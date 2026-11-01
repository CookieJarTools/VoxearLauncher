//! What the commands send to the window, shaped for the UI: camelCase, with times and paths as text.

use serde::{Deserialize, Serialize};
use voxear_launcher::backup::Backup;
use voxear_launcher::new_project::NewProject;
use voxear_launcher::upgrade::{Direction, Event, Report};
use voxear_launcher::{Channel, Install, InstallKind, Progress, Project, ProjectStatus, Release};

/// A command's error. `cancelled` tells a cancelled install apart from a failed one.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub message: String,
    pub cancelled: bool,
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        let cancelled = error.is::<voxear_launcher::store::Cancelled>();
        Self { message: format!("{error:#}"), cancelled }
    }
}

pub type Reply<T> = Result<T, Failure>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    /// The launcher's own folder.
    pub home: String,
    /// The user's home folder, for showing paths with `~`.
    pub user_home: String,
    pub settings_file: String,
    pub default_location: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallDto {
    pub version: String,
    pub channel: Channel,
    pub path: String,
    pub located: bool,
    pub missing: bool,
    pub installed: Option<String>,
    /// Optional parts, such as the manual: what's in the install, and what its release offers. Filled in by
    /// `list_installs`.
    pub modules: Vec<InstallModuleDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallModuleDto {
    pub name: String,
    pub title: String,
    pub installed: bool,
    /// The release offers it, so it can be added.
    pub available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDto {
    pub name: String,
    pub title: String,
    pub size: u64,
}

impl From<&Install> for InstallDto {
    fn from(install: &Install) -> Self {
        Self {
            version: install.version.to_string(),
            channel: install.channel,
            path: install.path.display().to_string(),
            located: install.kind == InstallKind::Located,
            missing: install.missing,
            installed: install.installed.map(|time| time.to_string()),
            modules: Vec::new(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDto {
    pub version: String,
    pub channel: Channel,
    pub published: String,
    pub notes: Option<String>,
    pub size: u64,
    pub installed: bool,
    pub needs_newer_launcher: bool,
    pub modules: Vec<ModuleDto>,
}

impl ReleaseDto {
    pub fn new(release: &Release, size: u64, installed: bool) -> Self {
        Self {
            version: release.version.to_string(),
            channel: release.channel,
            published: release.published.to_string(),
            notes: release.notes.clone(),
            size,
            installed,
            needs_newer_launcher: release.needs_newer_launcher(),
            modules: release
                .modules
                .iter()
                .map(|module| ModuleDto { name: module.name.clone(), title: module.title.clone(), size: module.size })
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleasesDto {
    /// Where releases are listed, or `None` when no source is set yet.
    pub source: Option<String>,
    pub releases: Vec<ReleaseDto>,
    /// Why the saved list was shown instead of a fresh one.
    pub stale: Option<String>,
    /// Nothing has been published at the source yet.
    pub unpublished: bool,
}

#[derive(Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum ProgressDto {
    Downloading { done: u64, total: u64 },
    Verifying,
    Unpacking { done: u64, total: u64 },
    Finishing,
}

impl From<Progress> for ProgressDto {
    fn from(progress: Progress) -> Self {
        match progress {
            Progress::Downloading { done, total } => ProgressDto::Downloading { done, total },
            Progress::Verifying => ProgressDto::Verifying,
            Progress::Unpacking { done, total } => ProgressDto::Unpacking { done, total },
            Progress::Finishing => ProgressDto::Finishing,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub path: String,
    pub name: String,
    pub engine: Option<String>,
    pub status: StatusDto,
    pub favourite: bool,
    pub added: String,
    pub opened: Option<String>,
}

impl From<&Project> for ProjectDto {
    fn from(project: &Project) -> Self {
        let status = match &project.status {
            ProjectStatus::Ready => StatusDto::Ready,
            ProjectStatus::EngineMissing(version) => StatusDto::EngineMissing { version: version.to_string() },
            ProjectStatus::Unpinned => StatusDto::Unpinned,
            ProjectStatus::Missing => StatusDto::Missing,
            ProjectStatus::Unreadable(reason) => StatusDto::Unreadable { reason: reason.clone() },
        };
        Self {
            path: project.entry.path.display().to_string(),
            name: project.entry.name.clone(),
            engine: project.engine.as_ref().map(ToString::to_string),
            status,
            favourite: project.entry.favourite,
            added: project.entry.added.to_string(),
            opened: project.entry.opened.map(|time| time.to_string()),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StatusDto {
    Ready,
    EngineMissing { version: String },
    Unpinned,
    Missing,
    Unreadable { reason: String },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedDto {
    /// A located install that was forgotten, its files left alone.
    pub forgotten: bool,
}

/// The new-project form, as the window sends it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProjectForm {
    pub name: String,
    pub location: String,
    pub template: String,
    pub setup: Option<Vec<String>>,
    pub ide: Option<String>,
    pub git: bool,
    pub modding: bool,
    pub steam_app_id: Option<u32>,
}

impl From<NewProjectForm> for NewProject {
    fn from(form: NewProjectForm) -> Self {
        let mut project = NewProject::new(form.name, form.location, form.template);
        project.setup = form.setup;
        project.ide = form.ide.filter(|ide| !ide.is_empty());
        project.git = form.git;
        project.modding = form.modding;
        project.steam_app_id = form.steam_app_id;
        project
    }
}

/// Whether a newer launcher is out.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateStatusDto {
    /// This build can't update itself, and why.
    NotSetUp {
        reason: String,
    },
    NothingPublished,
    UpToDate {
        version: String,
    },
    Available {
        version: String,
        notes: Option<String>,
        date: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDto {
    pub done: u64,
    pub total: Option<u64>,
}

/// The engine versions a project could move to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeChoicesDto {
    pub name: String,
    pub current: Option<String>,
    pub choices: Vec<UpgradeChoiceDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeChoiceDto {
    pub version: String,
    pub channel: Channel,
    /// `pin`, `upgrade`, `downgrade`, or `current` for the version it is on.
    pub direction: &'static str,
    pub missing: bool,
}

impl UpgradeChoiceDto {
    pub fn new(install: &Install, current: Option<&semver::Version>) -> Self {
        let direction = match Direction::of(current, &install.version) {
            Some(Direction::Pin) => "pin",
            Some(Direction::Upgrade) => "upgrade",
            Some(Direction::Downgrade) => "downgrade",
            None => "current",
        };
        Self { version: install.version.to_string(), channel: install.channel, direction, missing: install.missing }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNoteDto {
    pub version: String,
    pub notes: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpgradeEventDto {
    BackingUp { done: u64, total: u64 },
    Step { message: String },
    Change { message: String },
}

impl From<Event> for UpgradeEventDto {
    fn from(event: Event) -> Self {
        match event {
            Event::BackingUp { done, total } => UpgradeEventDto::BackingUp { done, total },
            Event::Step(message) => UpgradeEventDto::Step { message },
            Event::Change(message) => UpgradeEventDto::Change { message },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupDto {
    pub archive: String,
    pub project: String,
    pub path: String,
    pub from: Option<String>,
    pub to: String,
    pub created: String,
    pub size: u64,
    pub files: u64,
    pub skipped: Vec<String>,
    pub changes: Vec<String>,
}

impl From<&Backup> for BackupDto {
    fn from(backup: &Backup) -> Self {
        Self {
            archive: backup.archive.display().to_string(),
            project: backup.project.clone(),
            path: backup.path.display().to_string(),
            from: backup.from.as_ref().map(ToString::to_string),
            to: backup.to.to_string(),
            created: backup.created.to_string(),
            size: backup.size,
            files: backup.files,
            skipped: backup.skipped.clone(),
            changes: backup.changes.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeReportDto {
    pub from: Option<String>,
    pub to: String,
    pub backup: BackupDto,
    pub changes: Vec<String>,
    pub migrated: bool,
}

impl From<&Report> for UpgradeReportDto {
    fn from(report: &Report) -> Self {
        Self {
            from: report.from.as_ref().map(ToString::to_string),
            to: report.to.to_string(),
            backup: BackupDto::from(&report.backup),
            changes: report.changes.clone(),
            migrated: report.migrated,
        }
    }
}
