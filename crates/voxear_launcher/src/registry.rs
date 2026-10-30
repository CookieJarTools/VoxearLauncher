//! The project list (`projects.toml`). It only records where projects are and when they were opened; the name
//! and engine version are read from each project's `voxear.toml`, so the list never disagrees with the
//! project. The name is kept too, so a project on a drive that isn't mounted still shows what it was.
//!
//! Removing a project from the list never touches its files.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jiff::Timestamp;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::project::{self, ProjectInfo};
use crate::store::Install;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub added: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favourite: bool,
}

impl Entry {
    /// When the project was last used: opened, or else added.
    pub fn last_used(&self) -> Timestamp {
        self.opened.unwrap_or(self.added)
    }
}

/// A project in the list, with what its `voxear.toml` says now.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub entry: Entry,
    pub engine: Option<Version>,
    pub status: ProjectStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProjectStatus {
    /// Its engine version is installed.
    Ready,
    /// It is pinned to a version that isn't installed; the launcher offers to install it.
    EngineMissing(Version),
    /// It has no `engine` in `voxear.toml` yet.
    Unpinned,
    /// The folder or its `voxear.toml` has gone.
    Missing,
    /// `voxear.toml` is there but can't be read.
    Unreadable(String),
}

#[derive(Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default, rename = "project")]
    projects: Vec<Entry>,
}

pub struct Registry {
    file: PathBuf,
    entries: Vec<Entry>,
}

impl Registry {
    /// Loads the list, or starts an empty one if there's no file yet. Load again before changing the list if
    /// another launcher might have changed it since: saving replaces the file.
    pub fn load(file: impl Into<PathBuf>) -> Result<Self> {
        let file = file.into();
        let entries = match std::fs::read_to_string(&file) {
            Ok(text) => {
                toml::from_str::<RegistryFile>(&text)
                    .with_context(|| format!("{} isn't valid", file.display()))?
                    .projects
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("couldn't read {}", file.display())),
        };
        Ok(Self { file, entries })
    }

    pub fn save(&self) -> Result<()> {
        let file = RegistryFile { projects: self.entries.clone() };
        fsutil::write_atomic(&self.file, toml::to_string(&file)?.as_bytes())
    }

    /// The list, favourites first, then the most recently used.
    pub fn entries(&self) -> Vec<&Entry> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by(|a, b| b.favourite.cmp(&a.favourite).then(b.last_used().cmp(&a.last_used())));
        entries
    }

    pub fn find(&self, path: &Path) -> Option<&Entry> {
        let path = fsutil::absolute(&project::project_root(path)).ok()?;
        self.entries.iter().find(|entry| fsutil::same_path(&entry.path, &path))
    }

    /// Adds a project folder. Adding one that's already listed refreshes its name instead.
    pub fn add(&mut self, path: &Path) -> Result<&Entry> {
        let root = fsutil::absolute(&project::project_root(path))?;
        let info = ProjectInfo::read(&root)?;
        let index = match self.position(&root) {
            Some(index) => {
                self.entries[index].name = info.name;
                index
            }
            None => {
                let entry =
                    Entry { path: root, name: info.name, added: Timestamp::now(), opened: None, favourite: false };
                self.entries.push(entry);
                self.entries.len() - 1
            }
        };
        Ok(&self.entries[index])
    }

    /// Takes a project off the list. Its files are left alone. Returns whether it was listed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.entries.len();
        if let Ok(root) = fsutil::absolute(&project::project_root(path)) {
            self.entries.retain(|entry| !fsutil::same_path(&entry.path, &root));
        }
        self.entries.len() != before
    }

    /// Records that a project was just opened, adding it if it isn't listed.
    pub fn touch(&mut self, path: &Path) -> Result<()> {
        self.add(path)?;
        let root = fsutil::absolute(&project::project_root(path))?;
        let index = self.position(&root).expect("add just listed it");
        self.entries[index].opened = Some(Timestamp::now());
        Ok(())
    }

    pub fn set_favourite(&mut self, path: &Path, favourite: bool) -> bool {
        let Ok(root) = fsutil::absolute(&project::project_root(path)) else { return false };
        match self.position(&root) {
            Some(index) => {
                self.entries[index].favourite = favourite;
                true
            }
            None => false,
        }
    }

    /// The list with each project's current state, given the engine versions installed.
    pub fn projects(&self, installs: &[Install]) -> Vec<Project> {
        self.entries().into_iter().map(|entry| status(entry, installs)).collect()
    }

    /// Projects pinned to `version`, for warning before it is removed.
    pub fn using(&self, version: &Version) -> Vec<Project> {
        self.projects(&[]).into_iter().filter(|project| project.engine.as_ref() == Some(version)).collect()
    }

    fn position(&self, root: &Path) -> Option<usize> {
        self.entries.iter().position(|entry| fsutil::same_path(&entry.path, root))
    }
}

fn status(entry: &Entry, installs: &[Install]) -> Project {
    let project = |engine, status| Project { entry: entry.clone(), engine, status };
    if !entry.path.join(project::PROJECT_FILE).is_file() {
        return project(None, ProjectStatus::Missing);
    }
    match ProjectInfo::read(&entry.path) {
        Err(e) => project(None, ProjectStatus::Unreadable(format!("{e:#}"))),
        Ok(ProjectInfo { engine: None, .. }) => project(None, ProjectStatus::Unpinned),
        Ok(ProjectInfo { engine: Some(version), .. }) => {
            let installed = installs.iter().any(|install| install.version == version && !install.missing);
            let status = if installed { ProjectStatus::Ready } else { ProjectStatus::EngineMissing(version.clone()) };
            project(Some(version), status)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Channel;
    use crate::store::InstallKind;

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self { dir: tempfile::tempdir().unwrap() }
        }

        fn registry(&self) -> Registry {
            Registry::load(self.dir.path().join("projects.toml")).unwrap()
        }

        fn project(&self, folder: &str, toml: &str) -> PathBuf {
            let path = self.dir.path().join(folder);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("voxear.toml"), toml).unwrap();
            path
        }

        fn pinned(&self, folder: &str, version: &str) -> PathBuf {
            self.project(folder, &format!("[project]\nname = \"{folder}\"\nengine = \"{version}\"\n"))
        }
    }

    fn install(version: &str) -> Install {
        Install {
            version: Version::parse(version).unwrap(),
            channel: Channel::Stable,
            path: PathBuf::from("/engines").join(version),
            kind: InstallKind::Managed,
            installed: None,
            missing: false,
        }
    }

    #[test]
    fn adds_projects_and_keeps_them_after_saving() {
        let fixture = Fixture::new();
        let orbit = fixture.pinned("Orbit", "0.3.0");
        let mut registry = fixture.registry();
        let entry = registry.add(&orbit).unwrap();
        assert_eq!(entry.name, "Orbit");
        assert_eq!(entry.path, orbit);
        registry.save().unwrap();

        let again = fixture.registry();
        assert_eq!(again.entries().len(), 1);
        assert_eq!(again.entries()[0].name, "Orbit");
    }

    #[test]
    fn adding_twice_keeps_one_entry_and_refreshes_the_name() {
        let fixture = Fixture::new();
        let orbit = fixture.pinned("Orbit", "0.3.0");
        let mut registry = fixture.registry();
        registry.add(&orbit).unwrap();
        fixture.project("Orbit", "[project]\nname = \"Orbit Deluxe\"\n");
        registry.add(&orbit.join("voxear.toml")).unwrap();
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].name, "Orbit Deluxe");
    }

    #[test]
    fn refuses_folders_that_are_not_projects() {
        let fixture = Fixture::new();
        let mut registry = fixture.registry();
        assert!(registry.add(fixture.dir.path()).is_err());
        assert!(registry.entries().is_empty());
    }

    #[test]
    fn removing_forgets_the_project_but_keeps_its_files() {
        let fixture = Fixture::new();
        let orbit = fixture.pinned("Orbit", "0.3.0");
        let mut registry = fixture.registry();
        registry.add(&orbit).unwrap();
        assert!(registry.remove(&orbit));
        assert!(!registry.remove(&orbit));
        assert!(registry.entries().is_empty());
        assert!(orbit.join("voxear.toml").is_file());
    }

    #[test]
    fn lists_favourites_first_then_the_most_recently_used() {
        let fixture = Fixture::new();
        let mut registry = fixture.registry();
        for name in ["A", "B", "C"] {
            registry.add(&fixture.pinned(name, "0.3.0")).unwrap();
        }
        registry.touch(&fixture.dir.path().join("A")).unwrap();
        registry.set_favourite(&fixture.dir.path().join("C"), true);
        let names: Vec<_> = registry.entries().iter().map(|entry| entry.name.clone()).collect();
        assert_eq!(names, ["C", "A", "B"]);
    }

    #[test]
    fn reports_what_each_project_needs() {
        let fixture = Fixture::new();
        let mut registry = fixture.registry();
        let ready = fixture.pinned("Ready", "0.3.0");
        let needs = fixture.pinned("Needs", "0.2.0");
        let loose = fixture.project("Loose", "[project]\nname = \"Loose\"\n");
        let broken = fixture.pinned("Broken", "0.3.0");
        let gone = fixture.pinned("Gone", "0.3.0");
        for path in [&ready, &needs, &loose, &broken, &gone] {
            registry.add(path).unwrap();
        }
        fixture.project("Broken", "[project]\nname = \"Broken\"\nengine = \"soon\"\n");
        std::fs::remove_dir_all(&gone).unwrap();

        let projects = registry.projects(&[install("0.3.0")]);
        let status = |name: &str| projects.iter().find(|p| p.entry.name == name).unwrap().status.clone();
        assert_eq!(status("Ready"), ProjectStatus::Ready);
        assert_eq!(status("Needs"), ProjectStatus::EngineMissing(Version::new(0, 2, 0)));
        assert_eq!(status("Loose"), ProjectStatus::Unpinned);
        assert!(matches!(status("Broken"), ProjectStatus::Unreadable(_)));
        assert_eq!(status("Gone"), ProjectStatus::Missing);
    }

    #[test]
    fn finds_projects_pinned_to_a_version() {
        let fixture = Fixture::new();
        let mut registry = fixture.registry();
        registry.add(&fixture.pinned("Old", "0.2.0")).unwrap();
        registry.add(&fixture.pinned("New", "0.3.0")).unwrap();
        let using: Vec<_> = registry.using(&Version::new(0, 2, 0)).into_iter().map(|p| p.entry.name).collect();
        assert_eq!(using, ["Old"]);
    }
}
