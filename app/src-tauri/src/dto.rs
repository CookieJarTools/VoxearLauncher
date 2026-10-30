//! What the commands send to the window, shaped for the UI: camelCase, with times and paths as text.

use serde::{Deserialize, Serialize};
use voxear_launcher::new_project::NewProject;
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
