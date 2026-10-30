//! The commands the window calls. Each one runs `voxear_launcher` on a worker thread, so a slow disk or a
//! long download never freezes the window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use semver::Version;
use tauri::State;
use tauri::ipc::Channel;
use voxear_launcher::manifest::SIGNATURE_SUFFIX;
use voxear_launcher::new_project::{self, NewProject, Offer};
use voxear_launcher::{Layout, Platform, Registry, Removed, Selector, Settings, Store, project, releases};

use crate::dto::*;

/// What every command shares: where the launcher's files are, and the cancel flags of running installs.
pub struct Launcher {
    layout: Layout,
    platform: Platform,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Launcher {
    pub fn new() -> Result<Self> {
        let platform = Platform::current().ok_or_else(|| anyhow!("Voxear isn't built for this platform"))?;
        Ok(Self::with_layout(Layout::from_env()?, platform))
    }

    pub fn with_layout(layout: Layout, platform: Platform) -> Self {
        Self { layout, platform, cancels: Mutex::default() }
    }

    fn store(&self) -> Store {
        Store::new(self.layout.clone(), self.platform)
    }

    fn settings(&self) -> Result<Settings> {
        Settings::load(&self.layout)
    }

    fn registry(&self) -> Result<Registry> {
        Registry::load(self.layout.projects_file())
    }

    fn cancel_flag(&self, version: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels.lock().expect("the lock is never poisoned").insert(version.to_owned(), flag.clone());
        flag
    }

    fn forget_cancel_flag(&self, version: &str) {
        self.cancels.lock().expect("the lock is never poisoned").remove(version);
    }

    fn default_location(&self) -> PathBuf {
        let settings = self.settings().unwrap_or_default();
        match settings.project_location {
            Some(location) => PathBuf::from(location),
            None => dirs::home_dir().unwrap_or_default().join("VoxearProjects"),
        }
    }
}

type Shared<'a> = State<'a, Arc<Launcher>>;

/// Runs `work` on a worker thread.
async fn blocking<T: Send + 'static>(
    launcher: &Shared<'_>,
    work: impl FnOnce(&Launcher) -> Result<T> + Send + 'static,
) -> Reply<T> {
    let launcher = launcher.inner().clone();
    let joined = tauri::async_runtime::spawn_blocking(move || work(&launcher)).await;
    joined.map_err(|e| Failure { message: e.to_string(), cancelled: false })?.map_err(Failure::from)
}

#[tauri::command]
pub async fn app_info(launcher: Shared<'_>) -> Reply<AppInfo> {
    blocking(&launcher, |launcher| {
        Ok(AppInfo {
            version: voxear_launcher::launcher_version().to_string(),
            platform: launcher.platform.to_string(),
            home: launcher.layout.root().display().to_string(),
            user_home: dirs::home_dir().unwrap_or_default().display().to_string(),
            settings_file: launcher.layout.settings_file().display().to_string(),
            default_location: launcher.default_location().display().to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn list_installs(launcher: Shared<'_>) -> Reply<Vec<InstallDto>> {
    blocking(&launcher, |launcher| Ok(launcher.store().list()?.iter().map(InstallDto::from).collect())).await
}

#[tauri::command]
pub async fn list_releases(launcher: Shared<'_>) -> Reply<ReleasesDto> {
    blocking(&launcher, |launcher| {
        let settings = launcher.settings()?;
        let Some(source) = settings.releases_url() else {
            return Ok(ReleasesDto { source: None, releases: Vec::new(), stale: None });
        };
        let loaded = releases::load(&source, &settings.trusted_keys()?, &launcher.layout)?;
        let installs = launcher.store().list()?;
        let releases = loaded
            .manifest
            .available(launcher.platform)
            .into_iter()
            .map(|release| {
                let size = release.download(launcher.platform).map_or(0, |download| download.size);
                let installed = installs.iter().any(|install| install.version == release.version);
                ReleaseDto::new(release, size, installed)
            })
            .collect();
        Ok(ReleasesDto { source: Some(source), releases, stale: loaded.stale })
    })
    .await
}

/// Installs the release `version` from the manifest, sending progress as it goes.
#[tauri::command]
pub async fn install(launcher: Shared<'_>, version: String, on_progress: Channel<ProgressDto>) -> Reply<InstallDto> {
    blocking(&launcher, move |launcher| {
        let settings = launcher.settings()?;
        let source = settings.releases_url().context("no release source is set; add one in Settings")?;
        let keys = settings.trusted_keys()?;
        let loaded = releases::load(&source, &keys, &launcher.layout)?;
        let selector: Selector = version.parse()?;
        let release = loaded
            .manifest
            .select(&selector, launcher.platform)
            .ok_or_else(|| anyhow!("Voxear {version} isn't in the release list"))?;
        let cancel = launcher.cancel_flag(&version);
        let mut progress = |step| _ = on_progress.send(ProgressDto::from(step));
        let result = launcher.store().install(release, &loaded.url, &keys, &mut progress, &cancel);
        launcher.forget_cancel_flag(&version);
        Ok(InstallDto::from(&result?))
    })
    .await
}

#[tauri::command]
pub fn cancel_install(launcher: Shared<'_>, version: String) {
    if let Some(flag) = launcher.cancels.lock().expect("the lock is never poisoned").get(&version) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Installs an engine archive from disk; its `.minisig` must sit next to it.
#[tauri::command]
pub async fn install_file(launcher: Shared<'_>, path: String, on_progress: Channel<ProgressDto>) -> Reply<InstallDto> {
    blocking(&launcher, move |launcher| {
        let signature_path = format!("{path}{SIGNATURE_SUFFIX}");
        let signature = std::fs::read_to_string(&signature_path)
            .with_context(|| format!("there's no signature next to it (expected {signature_path})"))?;
        let keys = launcher.settings()?.trusted_keys()?;
        let mut progress = |step| _ = on_progress.send(ProgressDto::from(step));
        let install = launcher.store().install_archive(Path::new(&path), &signature, &keys, &mut progress)?;
        Ok(InstallDto::from(&install))
    })
    .await
}

#[tauri::command]
pub async fn locate(launcher: Shared<'_>, path: String) -> Reply<InstallDto> {
    blocking(&launcher, move |launcher| Ok(InstallDto::from(&launcher.store().locate(Path::new(&path))?))).await
}

/// The names of projects pinned to `version`, to warn before it is removed.
#[tauri::command]
pub async fn projects_using(launcher: Shared<'_>, version: String) -> Reply<Vec<String>> {
    blocking(&launcher, move |launcher| {
        let version = Version::parse(&version)?;
        Ok(launcher.registry()?.using(&version).into_iter().map(|project| project.entry.name).collect())
    })
    .await
}

#[tauri::command]
pub async fn uninstall(launcher: Shared<'_>, version: String) -> Reply<RemovedDto> {
    blocking(&launcher, move |launcher| {
        let removed = launcher.store().remove(&Version::parse(&version)?)?;
        Ok(RemovedDto { forgotten: matches!(removed, Removed::Forgotten(_)) })
    })
    .await
}

/// How much disk an install uses, in bytes.
#[tauri::command]
pub async fn install_size(launcher: Shared<'_>, path: String) -> Reply<u64> {
    blocking(&launcher, move |_| Ok(folder_size(Path::new(&path)))).await
}

fn folder_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
            Ok(_) => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

#[tauri::command]
pub async fn list_projects(launcher: Shared<'_>) -> Reply<Vec<ProjectDto>> {
    blocking(&launcher, |launcher| {
        let installs = launcher.store().list()?;
        Ok(launcher.registry()?.projects(&installs).iter().map(ProjectDto::from).collect())
    })
    .await
}

#[tauri::command]
pub async fn add_project(launcher: Shared<'_>, path: String) -> Reply<String> {
    blocking(&launcher, move |launcher| {
        let mut registry = launcher.registry()?;
        let name = registry.add(Path::new(&path))?.name.clone();
        registry.save()?;
        Ok(name)
    })
    .await
}

#[tauri::command]
pub async fn remove_project(launcher: Shared<'_>, path: String) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        let mut registry = launcher.registry()?;
        registry.remove(Path::new(&path));
        registry.save()
    })
    .await
}

#[tauri::command]
pub async fn set_favourite(launcher: Shared<'_>, path: String, favourite: bool) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        let mut registry = launcher.registry()?;
        registry.set_favourite(Path::new(&path), favourite);
        registry.save()
    })
    .await
}

/// Opens a project in the editor of the engine version it is pinned to.
#[tauri::command]
pub async fn open_project(launcher: Shared<'_>, path: String) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        let root = project::project_root(Path::new(&path));
        let info = project::ProjectInfo::read(&root)?;
        let version = info.engine.ok_or_else(|| anyhow!("{} isn't pinned to an engine version yet", info.name))?;
        let install = launcher
            .store()
            .get(&version)?
            .filter(|install| !install.missing)
            .ok_or_else(|| anyhow!("{} needs Voxear {version}, which isn't installed", info.name))?;
        project::open(&install, &root)?;
        let mut registry = launcher.registry()?;
        registry.touch(&root)?;
        registry.save()
    })
    .await
}

/// What an installed engine version offers for new projects.
#[tauri::command]
pub async fn describe_new(launcher: Shared<'_>, version: String) -> Reply<Offer> {
    blocking(&launcher, move |launcher| {
        let install = launcher
            .store()
            .get(&Version::parse(&version)?)?
            .ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
        new_project::describe(&install)
    })
    .await
}

/// Checks the name and folder while the form is filled in. Returns what's wrong, if anything.
#[tauri::command]
pub async fn check_new_project(launcher: Shared<'_>, form: NewProjectForm) -> Reply<Option<String>> {
    blocking(&launcher, move |_| Ok(NewProject::from(form).check(None).err().map(|e| format!("{e:#}")))).await
}

/// Creates a project with the engine version's own `vox new`, sending each step it reports, and adds it to
/// the list. Returns the project folder.
#[tauri::command]
pub async fn create_project(
    launcher: Shared<'_>,
    version: String,
    form: NewProjectForm,
    on_step: Channel<String>,
) -> Reply<String> {
    blocking(&launcher, move |launcher| {
        let install = launcher
            .store()
            .get(&Version::parse(&version)?)?
            .ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
        let project = NewProject::from(form);
        let offer = new_project::describe(&install)?;
        project.check(Some(&offer))?;
        let path = new_project::create(&install, &project, &mut |step| _ = on_step.send(step.to_owned()))?;

        let mut registry = launcher.registry()?;
        registry.add(&path)?;
        registry.save()?;
        let mut settings = launcher.settings()?;
        settings.project_location = Some(project.location.display().to_string());
        settings.save(&launcher.layout)?;
        Ok(path.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_settings(launcher: Shared<'_>) -> Reply<Settings> {
    blocking(&launcher, |launcher| launcher.settings()).await
}

#[tauri::command]
pub async fn save_settings(launcher: Shared<'_>, settings: Settings) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        settings.trusted_keys()?;
        settings.save(&launcher.layout)
    })
    .await
}
