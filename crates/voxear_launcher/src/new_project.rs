//! The new-project flow. The launcher never writes project files itself. It asks the chosen engine version's
//! `vox` what it offers (`vox new --describe --json`), builds the form from that, and then runs `vox new`, so
//! the launcher, the command line and the editor create identical projects. The contract is in
//! `docs/DESIGN.md`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::engine;
use crate::store::Install;

/// The `vox new --describe` format this launcher reads.
pub const FORMAT: u32 = 1;

/// What an engine version's `vox new` offers. Templates, setup items and IDEs come from the engine, so a newer
/// engine can add one without a launcher update.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub format: u32,
    pub templates: Vec<Template>,
    /// The parts of the default setup: lighting and sky, camera, input actions, a sample scene, quality presets.
    #[serde(default)]
    pub setup: Vec<Choice>,
    #[serde(default)]
    pub ides: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    /// `empty`, `3d`, `2d`, `first-person`, `third-person` or `top-down` in the first engine releases.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// An image for the template picker, relative to the engine install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<PathBuf>,
    /// The setup items this template turns on unless the user changes them.
    #[serde(default)]
    pub setup: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Offer {
    pub fn template(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|template| template.id == id)
    }
}

/// Everything the new-project form asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct NewProject {
    pub name: String,
    /// The folder the project folder is created in.
    pub location: PathBuf,
    pub template: String,
    /// The setup items to include. `None` uses the template's defaults; an empty list means none.
    pub setup: Option<Vec<String>>,
    pub ide: Option<String>,
    pub git: bool,
    pub modding: bool,
    pub steam_app_id: Option<u32>,
}

impl NewProject {
    /// A project from a template with its default setup and no extras.
    pub fn new(name: impl Into<String>, location: impl Into<PathBuf>, template: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            location: location.into(),
            template: template.into(),
            setup: None,
            ide: None,
            git: false,
            modding: false,
            steam_app_id: None,
        }
    }

    /// The project folder that will be created.
    pub fn path(&self) -> PathBuf {
        self.location.join(self.name.trim())
    }

    /// Catches what can be caught before running `vox new`, so the form can say what's wrong next to the field.
    /// `offer` checks the template, setup and IDE against what the engine version offers.
    pub fn check(&self, offer: Option<&Offer>) -> Result<()> {
        check_name(&self.name)?;
        let path = self.path();
        if path.exists() {
            let empty = path.read_dir().map(|mut entries| entries.next().is_none()).unwrap_or(false);
            ensure!(empty, "{} already exists", path.display());
        }
        ensure!(self.steam_app_id != Some(0), "a Steam App ID can't be 0");
        let Some(offer) = offer else { return Ok(()) };
        ensure!(offer.template(&self.template).is_some(), "this engine version has no `{}` template", self.template);
        for item in self.setup.iter().flatten() {
            ensure!(offer.setup.iter().any(|choice| &choice.id == item), "this engine version has no `{item}` setup");
        }
        if let Some(ide) = &self.ide {
            ensure!(offer.ides.iter().any(|choice| &choice.id == ide), "this engine version can't set up `{ide}`");
        }
        Ok(())
    }

    /// The arguments for `vox`.
    pub fn vox_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["new".into(), self.path().into(), "--name".into(), self.name.trim().into()];
        args.extend(["--template".into(), self.template.clone().into()]);
        if let Some(setup) = &self.setup {
            let list = if setup.is_empty() { "none".to_owned() } else { setup.join(",") };
            args.extend(["--setup".into(), list.into()]);
        }
        if let Some(ide) = &self.ide {
            args.extend(["--ide".into(), ide.into()]);
        }
        if self.git {
            args.push("--git".into());
        }
        if self.modding {
            args.push("--modding".into());
        }
        if let Some(id) = self.steam_app_id {
            args.extend(["--steam-app-id".into(), id.to_string().into()]);
        }
        args.push("--json".into());
        args
    }
}

/// A line `vox new --json` prints while it works.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Step { message: String },
    Done { path: PathBuf },
    Error { message: String },
}

/// Asks an engine version what its `vox new` offers.
pub fn describe(install: &Install) -> Result<Offer> {
    let vox = install.vox();
    let output = engine::command(&install.path, &vox)
        .args(["new", "--describe", "--json"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("couldn't run {}", vox.display()))?;
    if !output.status.success() {
        bail!(
            "Voxear {} can't create projects: `vox new --describe` failed: {}",
            install.version,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let offer: Offer = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("Voxear {}'s `vox new --describe` printed something unexpected", install.version))?;
    ensure!(
        offer.format <= FORMAT,
        "Voxear {} describes new projects in format {}, but this launcher reads format {FORMAT}; update the launcher",
        install.version,
        offer.format
    );
    Ok(offer)
}

/// Creates a project with an engine version's `vox new`, passing each step it reports to `on_step`. Returns
/// the project folder.
pub fn create(install: &Install, project: &NewProject, on_step: &mut dyn FnMut(&str)) -> Result<PathBuf> {
    let mut done = None;
    let mut failure = None;
    let mut command = engine::command(&install.path, &install.vox());
    let finished = engine::run_events(command.args(project.vox_args()), &mut |event| match event {
        Event::Step { message } => on_step(&message),
        Event::Done { path } => done = Some(path),
        Event::Error { message } => failure = Some(message),
    })?;
    if let Some(message) = failure {
        bail!("{message}");
    }
    ensure!(finished.success, "`vox new` failed: {}", finished.stderr.trim());
    done.ok_or_else(|| anyhow!("`vox new` finished without saying where it created the project"))
}

/// Project names become folder names, so they follow the rules of both Linux and Windows.
pub fn check_name(name: &str) -> Result<()> {
    let name = name.trim();
    ensure!(!name.is_empty(), "the project needs a name");
    ensure!(name.chars().count() <= 64, "the project name is longer than 64 characters");
    if let Some(bad) = name.chars().find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        bail!("project names can't contain `{bad}`");
    }
    ensure!(!name.chars().any(char::is_control), "project names can't contain control characters");
    ensure!(!name.ends_with('.'), "project names can't end with a full stop");
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    ensure!(!reserved, "`{name}` is a name Windows reserves for devices");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> Offer {
        serde_json::from_str(
            r#"{
                "format": 1,
                "templates": [
                    { "id": "empty", "name": "Empty" },
                    { "id": "3d", "name": "3D", "description": "A lit 3D scene", "thumbnail": "templates/3d/thumbnail.png",
                      "setup": ["lighting", "camera", "input", "sample-scene", "quality"] }
                ],
                "setup": [
                    { "id": "lighting", "name": "Lighting and sky" },
                    { "id": "camera", "name": "Camera" },
                    { "id": "input", "name": "Input actions" },
                    { "id": "sample-scene", "name": "Sample scene" },
                    { "id": "quality", "name": "Quality presets" }
                ],
                "ides": [
                    { "id": "rider", "name": "Rider" },
                    { "id": "visual-studio", "name": "Visual Studio" },
                    { "id": "vscode", "name": "VS Code" }
                ],
                "future_field": true
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn reads_what_an_engine_offers() {
        let offer = offer();
        assert_eq!(offer.templates.len(), 2);
        assert_eq!(offer.template("3d").unwrap().setup.len(), 5);
        assert_eq!(offer.ides[1].name, "Visual Studio");
    }

    #[test]
    fn builds_vox_new_arguments() {
        let mut project = NewProject::new(" Orbit ", "/games", "3d");
        assert_eq!(project.vox_args(), ["new", "/games/Orbit", "--name", "Orbit", "--template", "3d", "--json"]);

        project.setup = Some(vec!["lighting".into(), "camera".into()]);
        project.ide = Some("rider".into());
        project.git = true;
        project.modding = true;
        project.steam_app_id = Some(480);
        let args: Vec<_> = project.vox_args().into_iter().map(|arg| arg.into_string().unwrap()).collect();
        assert_eq!(
            args,
            [
                "new",
                "/games/Orbit",
                "--name",
                "Orbit",
                "--template",
                "3d",
                "--setup",
                "lighting,camera",
                "--ide",
                "rider",
                "--git",
                "--modding",
                "--steam-app-id",
                "480",
                "--json"
            ]
        );

        project.setup = Some(Vec::new());
        assert!(project.vox_args().windows(2).any(|pair| pair[0] == "--setup" && pair[1] == "none"));
    }

    #[test]
    fn checks_the_form_against_the_offer() {
        let dir = tempfile::tempdir().unwrap();
        let offer = offer();
        let mut project = NewProject::new("Orbit", dir.path(), "3d");
        project.check(Some(&offer)).unwrap();

        project.template = "racing".into();
        assert!(project.check(Some(&offer)).is_err());
        project.template = "3d".into();
        project.setup = Some(vec!["weather".into()]);
        assert!(project.check(Some(&offer)).is_err());
        project.setup = None;
        project.ide = Some("emacs".into());
        assert!(project.check(Some(&offer)).is_err());
        project.ide = None;
        project.steam_app_id = Some(0);
        assert!(project.check(Some(&offer)).is_err());
    }

    #[test]
    fn refuses_a_folder_that_is_already_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let project = NewProject::new("Orbit", dir.path(), "3d");
        std::fs::create_dir(project.path()).unwrap();
        project.check(None).unwrap();
        std::fs::write(project.path().join("notes.txt"), "mine").unwrap();
        assert!(project.check(None).is_err());
    }

    #[test]
    fn names_must_work_as_folder_names_everywhere() {
        for good in ["Orbit", "My Game 2", "skate.park", "Ünïcode"] {
            check_name(good).unwrap();
        }
        for bad in ["", "   ", "a/b", "a\\b", "what?", "trailing.", "CON", "com1", "lpt9.txt", &"x".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad:?} should be refused");
        }
    }

    /// The whole flow against a fake engine whose `vox` is a shell script.
    #[cfg(unix)]
    #[test]
    fn creates_a_project_with_the_engines_own_vox() {
        use std::sync::atomic::AtomicBool;

        use crate::channel::Channel;
        use crate::layout::Layout;
        use crate::platform::Platform;
        use crate::registry::{ProjectStatus, Registry};
        use crate::store::Store;
        use crate::testing::FakeReleases;

        let dir = tempfile::tempdir().unwrap();
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.3.0", Channel::Stable);
        let url = fake.write();
        let layout = Layout::new(dir.path().join("home"));
        let store = Store::new(layout.clone(), Platform::current().unwrap());
        let release = fake.manifest.find(&semver::Version::new(0, 3, 0)).unwrap();
        let install = store.install(release, &url, &fake.trusted_keys(), &mut |_| {}, &AtomicBool::new(false)).unwrap();

        let offer = describe(&install).unwrap();
        let ids: Vec<_> = offer.templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["empty", "3d", "2d", "first-person", "third-person", "top-down"]);

        let mut project = NewProject::new("Orbit", dir.path().join("games"), "third-person");
        project.git = true;
        project.check(Some(&offer)).unwrap();
        let mut steps = Vec::new();
        let path = create(&install, &project, &mut |step| steps.push(step.to_owned())).unwrap();
        assert_eq!(path, project.path());
        assert_eq!(steps[0], "Copying the third-person template");

        let mut registry = Registry::load(layout.projects_file()).unwrap();
        registry.add(&path).unwrap();
        let listed = registry.projects(&store.list().unwrap());
        assert_eq!(listed[0].engine, Some(semver::Version::new(0, 3, 0)), "vox new pins its own version");
        assert_eq!(listed[0].status, ProjectStatus::Ready);
    }

    #[test]
    fn reads_vox_new_events() {
        let step: Event = serde_json::from_str(r#"{"event":"step","message":"Copying the 3D template"}"#).unwrap();
        assert_eq!(step, Event::Step { message: "Copying the 3D template".into() });
        let done: Event = serde_json::from_str(r#"{"event":"done","path":"/games/Orbit"}"#).unwrap();
        assert_eq!(done, Event::Done { path: "/games/Orbit".into() });
    }
}
