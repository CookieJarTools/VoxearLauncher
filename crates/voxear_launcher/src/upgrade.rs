//! Moving a project to another engine version, or pinning one that has no version yet.
//!
//! The project is always backed up first. Then the target version's own `vox upgrade` migrates it and rewrites
//! the pin. With an engine that has no `vox upgrade` (its `engine.toml` doesn't list it), only the pin changes,
//! and the report says that nothing else was converted.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail, ensure};
use semver::Version;
use serde::Deserialize;

use crate::backup::{self, Backup};
use crate::engine::{self, EngineInfo};
use crate::layout::Layout;
use crate::project::{self, ProjectInfo};
use crate::store::Install;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The project has no version yet.
    Pin,
    Upgrade,
    Downgrade,
}

impl Direction {
    pub fn of(from: Option<&Version>, to: &Version) -> Option<Direction> {
        match from {
            None => Some(Direction::Pin),
            Some(from) if from == to => None,
            Some(from) if from < to => Some(Direction::Upgrade),
            Some(_) => Some(Direction::Downgrade),
        }
    }
}

/// How an upgrade is getting on.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    BackingUp { done: u64, total: u64 },
    Step(String),
    Change(String),
}

#[derive(Clone, Debug)]
pub struct Report {
    pub from: Option<Version>,
    pub to: Version,
    pub backup: Backup,
    /// What changed, as the engine described it.
    pub changes: Vec<String>,
    /// Whether the engine's own `vox upgrade` ran. When it didn't, only the pin changed.
    pub migrated: bool,
}

/// Moves the project at `project` to the engine version `target`. A move to an older version is refused
/// unless `allow_downgrade` is set, since projects don't always open in older versions.
pub fn upgrade(
    layout: &Layout,
    project: &Path,
    target: &Install,
    allow_downgrade: bool,
    on_event: &mut dyn FnMut(Event),
) -> Result<Report> {
    let root = project::project_root(project);
    let info = ProjectInfo::read(&root)?;
    let from = info.engine.clone();
    let to = target.version.clone();
    let Some(direction) = Direction::of(from.as_ref(), &to) else {
        bail!("{} is already on Voxear {to}", info.name);
    };
    if direction == Direction::Downgrade && !allow_downgrade {
        let from = from.as_ref().expect("a downgrade starts from a version");
        bail!(
            "Voxear {to} is older than {from}, which {} uses. Projects don't always open in older versions, so a \
             downgrade has to be asked for",
            info.name
        );
    }
    ensure!(!target.missing, "Voxear {to} was at {}, which isn't there now", target.path.display());
    let engine = EngineInfo::read(&target.path)?;

    let mut backup = {
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut progress = |done, total| {
            if done == total || last.elapsed() >= Duration::from_millis(80) {
                on_event(Event::BackingUp { done, total });
                last = Instant::now();
            }
        };
        backup::create(layout, &root, &info.name, from.as_ref(), &to, &mut progress)?
    };
    let keep_safe = |error: anyhow::Error| {
        anyhow!("{error:#}. Nothing is lost: the backup from before is at {}", backup.archive.display())
    };

    let (changes, migrated) = if engine.supports.upgrade.is_some() {
        (run_vox_upgrade(target, &root, from.as_ref(), on_event).map_err(keep_safe)?, true)
    } else {
        on_event(Event::Step("Changing the pinned version".to_owned()));
        project::pin(&root, &to).map_err(keep_safe)?;
        let change = format!("Pinned to Voxear {to}");
        on_event(Event::Change(change.clone()));
        (vec![change], false)
    };

    let after = ProjectInfo::read(&root).map_err(keep_safe)?;
    if after.engine.as_ref() != Some(&to) {
        return Err(keep_safe(anyhow!("`vox upgrade` finished, but voxear.toml isn't pinned to Voxear {to}")));
    }
    backup.changes = changes.clone();
    backup.save()?;
    Ok(Report { from, to, backup, changes, migrated })
}

/// A line `vox upgrade --json` prints while it works.
#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Line {
    Step { message: String },
    Change { message: String },
    Done {},
    Error { message: String },
}

fn run_vox_upgrade(
    target: &Install,
    root: &Path,
    from: Option<&Version>,
    on_event: &mut dyn FnMut(Event),
) -> Result<Vec<String>> {
    let mut command = engine::command(&target.path, &target.vox());
    command.arg("upgrade").arg(root);
    if let Some(from) = from {
        command.arg("--from").arg(from.to_string());
    }
    command.arg("--json");

    let mut changes = Vec::new();
    let mut failure = None;
    let finished = engine::run_events(&mut command, &mut |line| match line {
        Line::Step { message } => on_event(Event::Step(message)),
        Line::Change { message } => {
            on_event(Event::Change(message.clone()));
            changes.push(message);
        }
        Line::Done {} => {}
        Line::Error { message } => failure = Some(message),
    })?;
    if let Some(message) = failure {
        bail!("{message}");
    }
    ensure!(finished.success, "`vox upgrade` failed: {}", finished.stderr.trim());
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::channel::Channel;
    use crate::engine::Supports;
    use crate::store::InstallKind;

    struct Fixture {
        dir: tempfile::TempDir,
        layout: Layout,
        project: PathBuf,
    }

    impl Fixture {
        fn new(engine: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let layout = Layout::new(dir.path().join("home"));
            let project = dir.path().join("Orbit");
            fs::create_dir_all(project.join("Scenes")).unwrap();
            let pin = engine.map(|v| format!("engine = \"{v}\"\n")).unwrap_or_default();
            fs::write(
                project.join("voxear.toml"),
                format!("[project]\nname = \"Orbit\"\n{pin}scripts = \"S.csproj\"\n"),
            )
            .unwrap();
            fs::write(project.join("Scenes/Main.scene"), "scene").unwrap();
            Self { dir, layout, project }
        }

        /// An engine install made by hand, optionally without `vox upgrade`.
        fn engine(&self, version: &str, upgrade: bool) -> Install {
            let path = self.dir.path().join(format!("engine-{version}"));
            #[cfg(unix)]
            {
                let archive = crate::testing::engine_archive(
                    &Version::parse(version).unwrap(),
                    Channel::Stable,
                    crate::Platform::current().unwrap(),
                );
                crate::archive::unpack_reader(archive.as_slice(), &path, &mut |_| {}).unwrap();
            }
            let info = EngineInfo {
                format: 1,
                version: Version::parse(version).unwrap(),
                channel: Channel::Stable,
                platform: "linux-x86_64".to_owned(),
                commit: None,
                supports: Supports { new: Some(1), upgrade: upgrade.then_some(1) },
            };
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("engine.toml"), toml::to_string(&info).unwrap()).unwrap();
            Install {
                version: info.version,
                channel: Channel::Stable,
                path,
                kind: InstallKind::Located,
                installed: None,
                missing: false,
            }
        }

        fn pinned(&self) -> Option<Version> {
            ProjectInfo::read(&self.project).unwrap().engine
        }
    }

    #[test]
    fn works_out_the_direction() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert_eq!(Direction::of(None, &v("0.2.0")), Some(Direction::Pin));
        assert_eq!(Direction::of(Some(&v("0.2.0")), &v("0.3.0")), Some(Direction::Upgrade));
        assert_eq!(Direction::of(Some(&v("0.3.0")), &v("0.3.0-beta.1")), Some(Direction::Downgrade));
        assert_eq!(Direction::of(Some(&v("0.3.0")), &v("0.3.0")), None);
    }

    #[test]
    fn only_changes_the_pin_when_the_engine_has_no_upgrade() {
        let fixture = Fixture::new(Some("0.2.0"));
        let target = fixture.engine("0.3.0", false);
        let mut events = Vec::new();
        let report = upgrade(&fixture.layout, &fixture.project, &target, false, &mut |e| events.push(e)).unwrap();

        assert!(!report.migrated);
        assert_eq!(report.from, Some(Version::new(0, 2, 0)));
        assert_eq!(report.changes, ["Pinned to Voxear 0.3.0"]);
        assert_eq!(fixture.pinned(), Some(Version::new(0, 3, 0)));
        assert!(matches!(events.first(), Some(Event::BackingUp { .. })));
        assert!(report.backup.archive.is_file());
        assert_eq!(backup::list(&fixture.layout, None).unwrap()[0].changes, report.changes, "the backup remembers");
        let text = fs::read_to_string(fixture.project.join("voxear.toml")).unwrap();
        assert!(text.contains("scripts = \"S.csproj\""), "{text}");
    }

    #[test]
    fn pins_a_project_that_had_no_version() {
        let fixture = Fixture::new(None);
        let target = fixture.engine("0.2.0", false);
        let report = upgrade(&fixture.layout, &fixture.project, &target, false, &mut |_| {}).unwrap();
        assert_eq!(report.from, None);
        assert_eq!(fixture.pinned(), Some(Version::new(0, 2, 0)));
    }

    #[test]
    fn refuses_the_same_version() {
        let fixture = Fixture::new(Some("0.2.0"));
        let target = fixture.engine("0.2.0", false);
        let error = upgrade(&fixture.layout, &fixture.project, &target, false, &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("already on"), "{error}");
        assert!(backup::list(&fixture.layout, None).unwrap().is_empty(), "nothing to back up for");
    }

    #[test]
    fn downgrades_only_when_asked() {
        let fixture = Fixture::new(Some("0.3.0"));
        let target = fixture.engine("0.2.0", false);
        let error = upgrade(&fixture.layout, &fixture.project, &target, false, &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("has to be asked for"), "{error}");
        assert_eq!(fixture.pinned(), Some(Version::new(0, 3, 0)));

        upgrade(&fixture.layout, &fixture.project, &target, true, &mut |_| {}).unwrap();
        assert_eq!(fixture.pinned(), Some(Version::new(0, 2, 0)));
    }

    #[cfg(unix)]
    #[test]
    fn runs_the_engines_own_upgrade() {
        let fixture = Fixture::new(Some("0.2.0"));
        let target = fixture.engine("0.3.0", true);
        let mut events = Vec::new();
        let report = upgrade(&fixture.layout, &fixture.project, &target, false, &mut |e| events.push(e)).unwrap();

        assert!(report.migrated);
        assert_eq!(
            report.changes,
            ["Moved from Voxear 0.2.0 to Voxear 0.3.0", "Converted 2 scenes to the new scene format"]
        );
        assert!(events.contains(&Event::Step("Reading voxear.toml".to_owned())));
        assert_eq!(fixture.pinned(), Some(Version::new(0, 3, 0)));
    }

    #[cfg(unix)]
    #[test]
    fn points_at_the_backup_when_the_engine_fails() {
        let fixture = Fixture::new(Some("0.2.0"));
        fs::write(fixture.project.join("FAIL_UPGRADE"), "").unwrap();
        let target = fixture.engine("0.3.0", true);
        let error = upgrade(&fixture.layout, &fixture.project, &target, false, &mut |_| {}).unwrap_err().to_string();

        assert!(error.contains("asked to fail"), "{error}");
        let backup = &backup::list(&fixture.layout, None).unwrap()[0];
        assert!(error.contains(&backup.archive.display().to_string()), "{error}");
        assert_eq!(fixture.pinned(), Some(Version::new(0, 2, 0)));
        assert!(backup.changes.is_empty(), "a failed upgrade records no changes");
    }
}
