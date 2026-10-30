//! The commands the window calls. Each one runs `voxear_launcher` on a worker thread, so a slow disk or a
//! long download never freezes the window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use semver::Version;
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_updater::UpdaterExt;
use voxear_launcher::manifest::SIGNATURE_SUFFIX;
use voxear_launcher::new_project::{self, NewProject, Offer};
use voxear_launcher::{
    Layout, Platform, Registry, Removed, Selector, Settings, Store, backup, fetch, launcher_version, project, releases,
    self_update, upgrade,
};

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
            return Ok(ReleasesDto { source: None, releases: Vec::new(), stale: None, unpublished: false });
        };
        let loaded = match releases::load(&source, &settings.trusted_keys()?, &launcher.layout) {
            Ok(loaded) => loaded,
            Err(error) if fetch::is_not_found(&error) => {
                return Ok(ReleasesDto { source: Some(source), releases: Vec::new(), stale: None, unpublished: true });
            }
            Err(error) => return Err(error),
        };
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
        Ok(ReleasesDto { source: Some(source), releases, stale: loaded.stale, unpublished: false })
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

/// The installed versions a project could move to, and which way each one is.
#[tauri::command]
pub async fn upgrade_choices(launcher: Shared<'_>, path: String) -> Reply<UpgradeChoicesDto> {
    blocking(&launcher, move |launcher| {
        let info = project::ProjectInfo::read(&project::project_root(Path::new(&path)))?;
        let choices = launcher
            .store()
            .list()?
            .iter()
            .map(|install| UpgradeChoiceDto::new(install, info.engine.as_ref()))
            .collect();
        Ok(UpgradeChoicesDto { name: info.name, current: info.engine.map(|v| v.to_string()), choices })
    })
    .await
}

/// Release notes for the versions a project passes through. Empty, rather than an error, when the release list
/// can't be read: the notes are a nicety.
#[tauri::command]
pub async fn release_notes(launcher: Shared<'_>, from: Option<String>, to: String) -> Reply<Vec<ReleaseNoteDto>> {
    blocking(&launcher, move |launcher| {
        let from = from.map(|v| Version::parse(&v)).transpose()?;
        let to = Version::parse(&to)?;
        let settings = launcher.settings()?;
        let Some(source) = settings.releases_url() else { return Ok(Vec::new()) };
        let Ok(loaded) = releases::load(&source, &settings.trusted_keys()?, &launcher.layout) else {
            return Ok(Vec::new());
        };
        let notes = loaded.manifest.between(from.as_ref(), &to).into_iter().filter_map(|release| {
            release.notes.clone().map(|notes| ReleaseNoteDto { version: release.version.to_string(), notes })
        });
        Ok(notes.collect())
    })
    .await
}

/// Moves a project to an installed engine version, backing it up first, and sends each step as it goes.
#[tauri::command]
pub async fn upgrade_project(
    launcher: Shared<'_>,
    path: String,
    version: String,
    allow_downgrade: bool,
    on_event: Channel<UpgradeEventDto>,
) -> Reply<UpgradeReportDto> {
    blocking(&launcher, move |launcher| {
        let target = launcher
            .store()
            .get(&Version::parse(&version)?)?
            .ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
        let mut on_event = |event| _ = on_event.send(UpgradeEventDto::from(event));
        let report = upgrade::upgrade(&launcher.layout, Path::new(&path), &target, allow_downgrade, &mut on_event)?;
        Ok(UpgradeReportDto::from(&report))
    })
    .await
}

/// Backups, newest first: every one, or only those of the project at `path`.
#[tauri::command]
pub async fn list_backups(launcher: Shared<'_>, path: Option<String>) -> Reply<Vec<BackupDto>> {
    blocking(&launcher, move |launcher| {
        let path = path.map(|path| project::project_root(Path::new(&path)));
        Ok(backup::list(&launcher.layout, path.as_deref())?.iter().map(BackupDto::from).collect())
    })
    .await
}

/// Restores a backup into a new folder beside its project, adds that to the list, and returns it.
#[tauri::command]
pub async fn restore_backup(launcher: Shared<'_>, archive: String) -> Reply<String> {
    blocking(&launcher, move |launcher| {
        let backup = backup::find(&launcher.layout, Path::new(&archive))?;
        let restored = backup::restore(&backup)?;
        let mut registry = launcher.registry()?;
        registry.add(&restored)?;
        registry.save()?;
        Ok(restored.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn delete_backup(launcher: Shared<'_>, archive: String) -> Reply<()> {
    blocking(&launcher, move |launcher| backup::delete(&backup::find(&launcher.layout, Path::new(&archive))?)).await
}

/// A launcher update that was found and not installed yet.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<tauri_plugin_updater::Update>>);

/// Why this copy of the launcher can't update itself, if it can't.
fn why_no_self_update() -> Option<String> {
    if self_update::update_key().is_none() {
        return Some("This build has no launcher update key, so it can't check for updates.".to_owned());
    }
    if cfg!(debug_assertions) {
        return Some("Development builds don't update themselves.".to_owned());
    }
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        return Some(
            "Only the AppImage updates itself. This copy was installed some other way, so update it that way."
                .to_owned(),
        );
    }
    None
}

/// Checks the launcher's channel for a newer version. Tauri's updater fetches and checks the update itself; the
/// launcher looks first, so "nothing published yet" isn't reported as a failure.
#[tauri::command]
pub async fn check_launcher_update<R: Runtime>(
    app: AppHandle<R>,
    launcher: Shared<'_>,
    pending: State<'_, PendingUpdate>,
) -> Reply<UpdateStatusDto> {
    if let Some(reason) = why_no_self_update() {
        return Ok(UpdateStatusDto::NotSetUp { reason });
    }
    let url = blocking(&launcher, |launcher| {
        Ok(self_update::manifest_url(launcher.settings()?.launcher_channel.unwrap_or_default()))
    })
    .await?;
    let lookup = url.clone();
    let Some(manifest) = blocking(&launcher, move |_| self_update::fetch_manifest(&lookup)).await? else {
        return Ok(UpdateStatusDto::NothingPublished);
    };
    let current = launcher_version();
    if manifest.version <= current {
        return Ok(UpdateStatusDto::UpToDate { version: current.to_string() });
    }

    let fail = |error: tauri_plugin_updater::Error| Failure {
        message: format!("couldn't check for updates: {error}"),
        cancelled: false,
    };
    let endpoint = url.parse().map_err(|e| Failure { message: format!("{url} isn't a URL: {e}"), cancelled: false })?;
    let updater = app.updater_builder().endpoints(vec![endpoint]).map_err(fail)?.build().map_err(fail)?;
    let Some(update) = updater.check().await.map_err(fail)? else {
        return Ok(UpdateStatusDto::UpToDate { version: current.to_string() });
    };
    let status = UpdateStatusDto::Available {
        version: update.version.clone(),
        notes: update.body.clone(),
        date: update.date.map(|date| date.to_string()),
    };
    *pending.0.lock().expect("the lock is never poisoned") = Some(update);
    Ok(status)
}

/// Downloads the update found by the last check, verifies it, and installs it. On Windows the installer takes over
/// from here; elsewhere the new version starts on the next restart.
#[tauri::command]
pub async fn install_launcher_update(
    pending: State<'_, PendingUpdate>,
    on_progress: Channel<DownloadDto>,
) -> Reply<()> {
    let update = pending.0.lock().expect("the lock is never poisoned").take().ok_or_else(|| Failure {
        message: "there's no update to install; check for updates first".to_owned(),
        cancelled: false,
    })?;
    let mut done = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                _ = on_progress.send(DownloadDto { done, total });
            },
            || {},
        )
        .await
        .map_err(|error| Failure { message: format!("couldn't install the update: {error}"), cancelled: false })
}

#[tauri::command]
pub fn restart_launcher<R: Runtime>(app: AppHandle<R>) {
    app.restart()
}
