//! What the launcher reads from a project's `voxear.toml`: its name, and the engine version it is pinned to.
//! Everything else in the file belongs to the engine.

use std::path::{Path, PathBuf};
use std::process::Child;

use anyhow::{Context, Result, anyhow, ensure};
use semver::Version;
use serde::Deserialize;

use crate::engine;
use crate::store::Install;

pub const PROJECT_FILE: &str = "voxear.toml";

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectInfo {
    pub name: String,
    /// The engine version the project is pinned to. Projects made before pinning existed have none.
    pub engine: Option<Version>,
}

#[derive(Deserialize)]
struct ProjectFile {
    project: ProjectSection,
}

#[derive(Deserialize)]
struct ProjectSection {
    name: String,
    #[serde(default)]
    engine: Option<String>,
}

impl ProjectInfo {
    /// Reads a project folder, or its `voxear.toml` directly.
    pub fn read(path: &Path) -> Result<Self> {
        let file = project_file(path);
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("there's no Voxear project at {} (no {PROJECT_FILE})", path.display()))?;
        let parsed: ProjectFile = toml::from_str(&text).with_context(|| format!("{} isn't valid", file.display()))?;
        let engine = parsed
            .project
            .engine
            .map(|engine| {
                Version::parse(engine.trim())
                    .map_err(|_| anyhow!("`engine = \"{engine}\"` in {} isn't an engine version", file.display()))
            })
            .transpose()?;
        Ok(Self { name: parsed.project.name, engine })
    }
}

/// Opens a project in an engine version's editor, which finds its own files through `VOXEAR_HOME`.
pub fn open(install: &Install, project: &Path) -> Result<Child> {
    let editor = install.editor();
    ensure!(editor.is_file(), "Voxear {} has no editor (expected {})", install.version, editor.display());
    engine::command(&install.path, &editor)
        .arg("--project")
        .arg(project_root(project))
        .spawn()
        .with_context(|| format!("couldn't start {}", editor.display()))
}

/// The project folder for a path that may be the folder or its `voxear.toml`.
pub fn project_root(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|name| name == PROJECT_FILE) && path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        path.to_path_buf()
    }
}

fn project_file(path: &Path) -> PathBuf {
    if path.is_dir() { path.join(PROJECT_FILE) } else { path.to_path_buf() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PROJECT_FILE), text).unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    #[test]
    fn reads_the_name_and_pinned_engine() {
        let (_dir, path) =
            project("[project]\nname = \"Orbit\"\nengine = \"0.3.0\"\nscripts = \"Scripts/Orbit.csproj\"\n");
        let info = ProjectInfo::read(&path).unwrap();
        assert_eq!(info.name, "Orbit");
        assert_eq!(info.engine, Some(Version::new(0, 3, 0)));
        assert_eq!(ProjectInfo::read(&path.join(PROJECT_FILE)).unwrap(), info);
    }

    #[test]
    fn reads_projects_that_are_not_pinned_yet() {
        // The shape of samples/Hello in the engine repository today.
        let (_dir, path) = project("[project]\nname = \"Hello\"\n\n[[objs]]\nname = \"Spinner A\"\n");
        assert_eq!(ProjectInfo::read(&path).unwrap().engine, None);
    }

    #[test]
    fn explains_a_bad_engine_version() {
        let (_dir, path) = project("[project]\nname = \"Orbit\"\nengine = \"latest\"\n");
        let error = ProjectInfo::read(&path).unwrap_err().to_string();
        assert!(error.contains("isn't an engine version"), "{error}");
    }

    #[test]
    fn explains_a_folder_that_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let error = ProjectInfo::read(dir.path()).unwrap_err().to_string();
        assert!(error.contains("no Voxear project"), "{error}");
    }
}
