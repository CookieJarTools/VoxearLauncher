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
use voxear_launcher::manifest::SIGNATURE_SUFFIX;
use voxear_launcher::new_project::{self, NewProject, Offer};
use voxear_launcher::self_update::UpdateChannel;
use voxear_launcher::sources::{self, Ambiguous};
use voxear_launcher::{
    Catalog, InstallKind, Layout, Module, Platform, Registry, Removed, Selector, Settings, Source, Store, backup,
    project, self_install, upgrade, whisk,
};

use crate::dto::*;
use crate::updates;

/// What every command shares: where the launcher's files are, and the cancel flags of running installs.
pub struct Launcher {
    layout: Layout,
    platform: Platform,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Where the official releases are listed, when it isn't their usual place.
    official: Option<String>,
}

impl Launcher {
    pub fn new() -> Result<Self> {
        let platform = Platform::current().ok_or_else(|| anyhow!("Voxear isn't built for this platform"))?;
        Ok(Self::with_layout(Layout::from_env()?, platform))
    }

    pub fn with_layout(layout: Layout, platform: Platform) -> Self {
        Self { layout, platform, cancels: Mutex::default(), official: None }
    }

    /// Lists the official releases from `url` instead of their usual place, as the tests do.
    #[cfg(test)]
    pub fn with_official_releases(self, url: impl Into<String>) -> Self {
        Self { official: Some(url.into()), ..self }
    }

    fn store(&self) -> Store {
        Store::new(self.layout.clone(), self.platform)
    }

    fn settings(&self) -> Result<Settings> {
        Settings::load(&self.layout)
    }

    /// The channel launcher updates come from.
    pub fn channel(&self) -> Result<UpdateChannel> {
        Ok(self.settings()?.launcher_channel.unwrap_or_default())
    }

    fn registry(&self) -> Result<Registry> {
        Registry::load(self.layout.projects_file())
    }

    /// Every release list, the official one first.
    fn sources(&self, settings: &Settings) -> Result<Vec<Source>> {
        match &self.official {
            Some(url) => settings.sources_with(url),
            None => settings.sources(),
        }
    }

    /// Every release list, fetched and checked.
    fn catalog(&self, settings: &Settings) -> Result<Catalog> {
        Ok(Catalog::load(self.sources(settings)?, &self.layout))
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
            app_menu: AppMenuDto::from(updates::app_menu()),
        })
    })
    .await
}

#[tauri::command]
pub async fn list_installs(launcher: Shared<'_>) -> Reply<Vec<InstallDto>> {
    blocking(&launcher, |launcher| {
        // The saved release lists say which modules each version offers, without going online.
        let settings = launcher.settings()?;
        let catalog = Catalog::cached(launcher.sources(&settings)?, &launcher.layout);
        let official = launcher.official.clone().unwrap_or_else(voxear_launcher::settings::official_url);
        let installs = launcher.store().list()?;
        Ok(installs
            .iter()
            .map(|install| {
                let release = catalog.of_install(install).map(|listed| listed.release);
                // Each part the release offers, and any the install has anyway, such as one an older archive carried.
                let modules = Module::ALL
                    .into_iter()
                    .filter_map(|module| {
                        let offered = release.and_then(|release| release.module(module, launcher.platform));
                        let installed = install.has_module(module);
                        (offered.is_some() || installed).then(|| InstallModuleDto {
                            name: module,
                            title: offered.map_or_else(|| module.title().to_owned(), |offered| offered.title.clone()),
                            installed,
                            available: offered.is_some() && install.kind == InstallKind::Managed,
                        })
                    })
                    .collect();
                let source = install.releases.as_deref().and_then(|url| settings.source_name(url, &official));
                InstallDto { modules, source, ..InstallDto::from(install) }
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn list_releases(launcher: Shared<'_>) -> Reply<ReleasesDto> {
    blocking(&launcher, |launcher| {
        let settings = launcher.settings()?;
        let catalog = launcher.catalog(&settings)?;
        let installs = launcher.store().list()?;
        let releases = catalog
            .available(launcher.platform)
            .into_iter()
            .map(|listed| {
                let release = listed.release;
                let size = release.download(launcher.platform).map_or(0, |download| download.size);
                let installed = installs.iter().any(|install| install.version == release.version);
                let source = (!listed.source.official).then(|| listed.source.name.clone());
                ReleaseDto::new(release, launcher.platform, size, installed, source)
            })
            .collect();
        let sources = catalog.listings.iter().map(SourceDto::from).collect();
        Ok(ReleasesDto { sources, releases, nightlies: settings.nightlies_shown(&installs) })
    })
    .await
}

/// Installs the release `version`, from the release list called `source` or else whichever list publishes it, sending
/// progress as it goes.
#[tauri::command]
pub async fn install(
    launcher: Shared<'_>,
    version: String,
    source: Option<String>,
    without: Option<Vec<Module>>,
    on_progress: Channel<ProgressDto>,
) -> Reply<InstallDto> {
    // Leaving `without` out installs every module the release offers.
    let without = without.unwrap_or_default();
    blocking(&launcher, move |launcher| {
        let catalog = launcher.catalog(&launcher.settings()?)?;
        let selector: Selector = version.parse()?;
        let listed = catalog.select(&selector, launcher.platform, source.as_deref()).map_err(choose_in_the_list)?;
        let listed = listed.ok_or_else(|| not_listed(&catalog, &version))?;
        let cancel = launcher.cancel_flag(&version);
        let mut progress = |step| _ = on_progress.send(ProgressDto::from(step));
        let (release, keys) = (listed.release, &listed.source.keys);
        let result = launcher.store().install_without(release, listed.url, keys, &without, &mut progress, &cancel);
        launcher.forget_cancel_flag(&version);
        Ok(InstallDto::from(&result?))
    })
    .await
}

/// Says where to choose when two release lists publish different engines as one version.
fn choose_in_the_list(error: anyhow::Error) -> anyhow::Error {
    match error.downcast_ref::<Ambiguous>() {
        Some(ambiguous) => anyhow!("{ambiguous}; choose one in Install a version"),
        None => error,
    }
}

/// Why a version wasn't found: not listed, or in a list that couldn't be read.
fn not_listed(catalog: &Catalog, version: &str) -> anyhow::Error {
    let unread: Vec<String> = catalog
        .listings
        .iter()
        .filter_map(|listing| match &listing.state {
            sources::ListingState::Failed(error) => Some(format!("{}: {error:#}", listing.source.name)),
            _ => None,
        })
        .collect();
    if unread.is_empty() {
        anyhow!("Voxear {version} isn't in the release lists")
    } else {
        anyhow!("Voxear {version} isn't in the release lists that could be read; couldn't read {}", unread.join("; "))
    }
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
        settings.check()?;
        settings.save(&launcher.layout)
    })
    .await
}

/// Lists someone else's engine releases beside the official ones, once they have been read and their signature
/// checked, so a wrong address or key shows at once. Saved straight away.
#[tauri::command]
pub async fn add_custom_releases(
    launcher: Shared<'_>,
    address: String,
    name: Option<String>,
    key: Option<String>,
) -> Reply<AddedReleasesDto> {
    blocking(&launcher, move |launcher| {
        let mut settings = launcher.settings()?;
        let added = settings.add_custom(&address, name.as_deref(), key.as_deref())?.clone();
        let source = launcher.sources(&settings)?.into_iter().find(|source| source.name == added.name);
        let source = source.context("the releases were just added")?;
        let releases = sources::check(&source, &launcher.layout)?;
        settings.save(&launcher.layout)?;
        Ok(AddedReleasesDto { name: added.name, url: source.url, releases })
    })
    .await
}

/// Stops listing custom releases. Engines installed from them stay.
#[tauri::command]
pub async fn remove_custom_releases(launcher: Shared<'_>, name: String) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        let mut settings = launcher.settings()?;
        settings.remove_custom(&name)?;
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
        let catalog = launcher.catalog(&launcher.settings()?)?;
        // The notes come from the list that publishes the version moved to.
        let Some((_, loaded)) = catalog.loaded().find(|(_, loaded)| loaded.manifest.find(&to).is_some()) else {
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
pub struct PendingUpdate(Mutex<Option<updates::Found>>);

/// Checks the launcher's channel for a newer version.
#[tauri::command]
pub async fn check_launcher_update<R: Runtime>(
    app: AppHandle<R>,
    launcher: Shared<'_>,
    pending: State<'_, PendingUpdate>,
) -> Reply<UpdateStatusDto> {
    if let Some(reason) = updates::why_no_self_update() {
        return Ok(UpdateStatusDto::NotSetUp { reason });
    }
    let channel = blocking(&launcher, |launcher| launcher.channel()).await?;
    let status = match updates::look_for_update(&app, channel).await.map_err(Failure::from)? {
        updates::Lookup::NothingPublished => UpdateStatusDto::NothingPublished,
        updates::Lookup::UpToDate(version) => UpdateStatusDto::UpToDate { version: version.to_string() },
        updates::Lookup::Available(found) => {
            let status = UpdateStatusDto::Available {
                version: found.update.version.clone(),
                notes: found.update.body.clone(),
                date: found.update.date.map(|date| date.to_string()),
            };
            *pending.0.lock().expect("the lock is never poisoned") = Some(found);
            status
        }
    };
    Ok(status)
}

/// Downloads the update found by the last check, verifies it, and installs it. On Windows the setup program
/// takes over from here; elsewhere the new version starts on the next restart.
#[tauri::command]
pub async fn install_launcher_update(
    pending: State<'_, PendingUpdate>,
    on_progress: Channel<DownloadDto>,
) -> Reply<()> {
    let found = pending.0.lock().expect("the lock is never poisoned").take().ok_or_else(|| Failure {
        message: "there's no update to install; check for updates first".to_owned(),
        cancelled: false,
    })?;
    updates::install(found, |done, total| _ = on_progress.send(DownloadDto { done, total }))
        .await
        .map_err(Failure::from)
}

/// Restarts into the installed version. The exit is requested rather than forced, so the running window lets
/// go of its single-instance lock before the new one starts.
#[tauri::command]
pub fn restart_launcher<R: Runtime>(app: AppHandle<R>) {
    app.request_restart()
}

/// Which program to start once the launcher has closed, when it should reopen as another copy of itself.
#[derive(Default)]
pub struct Relaunch(pub Mutex<Option<PathBuf>>);

/// Installs the AppImage this launcher runs from into the app menu (see [`self_install`]), then reopens the
/// launcher from the installed copy.
#[tauri::command]
pub async fn install_launcher_app<R: Runtime>(app: AppHandle<R>, relaunch: State<'_, Relaunch>) -> Reply<()> {
    let source = updates::appimage()
        .ok_or_else(|| Failure { message: "only the AppImage can be installed".to_owned(), cancelled: false })?;
    let installed = tauri::async_runtime::spawn_blocking(move || {
        self_install::install(&source, &self_install::Places::from_env()?, updates::ICONS)
    })
    .await
    .map_err(|e| Failure { message: e.to_string(), cancelled: false })?
    .map_err(Failure::from)?;
    *relaunch.0.lock().expect("the lock is never poisoned") = Some(installed.program);
    app.exit(0);
    Ok(())
}

/// Adds an optional part, such as the manual or the Dedicated Server, to one of the launcher's installs: downloaded
/// and checked like the engine itself. Cancelled the same way as an install, under `<version>+<name>`.
#[tauri::command]
pub async fn add_module(
    launcher: Shared<'_>,
    version: String,
    name: Module,
    on_progress: Channel<ProgressDto>,
) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        let parsed = Version::parse(&version)?;
        let install = launcher.store().get(&parsed)?.ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
        let catalog = launcher.catalog(&launcher.settings()?)?;
        let listed = catalog
            .of_install(&install)
            .ok_or_else(|| anyhow!("Voxear {version} isn't in the release lists, so its {name} can't be found"))?;
        let key = format!("{version}+{}", name.name());
        let cancel = launcher.cancel_flag(&key);
        let mut progress = |step| _ = on_progress.send(ProgressDto::from(step));
        let (release, keys) = (listed.release, &listed.source.keys);
        let result = launcher.store().add_module(release, name, listed.url, keys, &mut progress, &cancel);
        launcher.forget_cancel_flag(&key);
        result.map(|_| ())
    })
    .await
}

/// Removes an optional part, such as the manual or the Dedicated Server, from one of the launcher's installs.
#[tauri::command]
pub async fn remove_module(launcher: Shared<'_>, version: String, name: Module) -> Reply<()> {
    blocking(&launcher, move |launcher| launcher.store().remove_module(&Version::parse(&version)?, name)).await
}

/// Where Whisk, the code editor, is, if it's installed.
#[tauri::command]
pub async fn whisk_status(launcher: Shared<'_>) -> Reply<WhiskDto> {
    blocking(&launcher, |launcher| Ok(WhiskDto::from(whisk::find(&launcher.layout).as_ref()))).await
}

/// Installs the newest Whisk, checked against Whisk's own key. It keeps itself up to date after that.
#[tauri::command]
pub async fn install_whisk(launcher: Shared<'_>, on_progress: Channel<ProgressDto>) -> Reply<WhiskDto> {
    blocking(&launcher, move |launcher| {
        let mut progress = |step| _ = on_progress.send(ProgressDto::from(step));
        let url = whisk::manifest_url();
        let installed = whisk::install(&launcher.layout, &url, whisk::WHISK_KEY, launcher.platform, &mut progress)?;
        Ok(WhiskDto::from(Some(&installed.whisk)))
    })
    .await
}

/// Opens Whisk, on a project's folder when one is given.
#[tauri::command]
pub async fn open_whisk(launcher: Shared<'_>, path: Option<String>) -> Reply<()> {
    blocking(&launcher, move |launcher| {
        let found = whisk::find(&launcher.layout).ok_or_else(|| anyhow!("Whisk isn't installed"))?;
        let folder = path.map(|path| project::project_root(Path::new(&path)));
        whisk::open(&found.path, folder.as_deref(), &[])
    })
    .await
}

/// Removes the Whisk the launcher installed; one installed some other way stays.
#[tauri::command]
pub async fn remove_whisk(launcher: Shared<'_>) -> Reply<WhiskDto> {
    blocking(&launcher, |launcher| {
        whisk::remove(&launcher.layout)?;
        Ok(WhiskDto::from(whisk::find(&launcher.layout).as_ref()))
    })
    .await
}
