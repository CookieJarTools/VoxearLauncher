//! The window's commands, called through Tauri's IPC exactly as the front end calls them: the same command
//! names, the same camelCase arguments, and JSON back. They run against a fake signed release.

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};
use voxear_launcher::testing::FakeReleases;
use voxear_launcher::{Channel, Layout, Platform};

use crate::commands::Launcher;

struct Fixture {
    dir: tempfile::TempDir,
    window: WebviewWindow<MockRuntime>,
    fake: FakeReleases,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.1.0", Channel::Stable);
        fake.add("0.2.0", Channel::Stable);
        fake.write();
        let launcher = Launcher::with_layout(Layout::new(dir.path().join("home")), Platform::current().unwrap());
        let app = crate::with_commands(mock_builder(), launcher).build(mock_context(noop_assets())).unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        Self { dir, window, fake }
    }

    fn call(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(&self.window, request).map(|body| body.deserialize::<Value>().unwrap())
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args).unwrap_or_else(|error| panic!("{cmd} failed: {error}"))
    }

    fn trust_the_fake_releases(&self) {
        let settings = json!({ "releases": self.fake.url(), "trusted_keys": [self.fake.key.public()] });
        self.ok("save_settings", json!({ "settings": settings }));
    }

    fn form(&self, name: &str) -> Value {
        json!({
            "name": name,
            "location": self.dir.path().join("games").display().to_string(),
            "template": "3d",
            "setup": ["lighting", "camera"],
            "ide": "rider",
            "git": true,
            "modding": false,
            "steamAppId": 480
        })
    }
}

/// A channel argument, as `@tauri-apps/api`'s `Channel` serializes itself.
fn channel(id: u32) -> Value {
    json!(format!("__CHANNEL__:{id}"))
}

#[test]
fn describes_the_app() {
    let fixture = Fixture::new();
    let info = fixture.ok("app_info", json!({}));
    for key in ["version", "platform", "home", "userHome", "settingsFile", "defaultLocation"] {
        assert!(info[key].is_string(), "app_info has {key}: {info}");
    }
}

#[test]
fn says_when_nothing_is_published_yet() {
    let fixture = Fixture::new();
    let nowhere = fixture.dir.path().join("unpublished").join("releases.toml").display().to_string();
    fixture.ok("save_settings", json!({ "settings": { "releases": nowhere } }));
    let releases = fixture.ok("list_releases", json!({}));
    assert_eq!(releases["unpublished"], true);
    assert_eq!(releases["releases"], json!([]));
    assert_eq!(fixture.ok("list_installs", json!({})), json!([]));
}

#[test]
fn refuses_settings_with_a_bad_key() {
    let fixture = Fixture::new();
    let error = fixture.call("save_settings", json!({ "settings": { "trusted_keys": ["nope"] } })).unwrap_err();
    assert_eq!(error["cancelled"], false);
    assert!(error["message"].as_str().unwrap().contains("trusted_keys"), "{error}");
}

#[test]
fn installs_and_uninstalls_a_release() {
    let fixture = Fixture::new();
    fixture.trust_the_fake_releases();

    let releases = fixture.ok("list_releases", json!({}));
    let versions: Vec<_> = releases["releases"].as_array().unwrap().iter().map(|r| r["version"].clone()).collect();
    assert_eq!(versions, [json!("0.2.0"), json!("0.1.0")]);
    assert_eq!(releases["releases"][0]["installed"], false);
    assert!(releases["releases"][0]["size"].as_u64().unwrap() > 0);

    let install = fixture.ok("install", json!({ "version": "stable", "onProgress": channel(1) }));
    assert_eq!(install["version"], "0.2.0");
    assert_eq!(install["channel"], "stable");
    assert_eq!(install["located"], false);
    assert!(install["installed"].is_string());

    let size = fixture.ok("install_size", json!({ "path": install["path"] }));
    assert!(size.as_u64().unwrap() > 0);
    assert_eq!(fixture.ok("list_releases", json!({}))["releases"][0]["installed"], true);

    assert_eq!(fixture.ok("uninstall", json!({ "version": "0.2.0" })), json!({ "forgotten": false }));
    assert_eq!(fixture.ok("list_installs", json!({})), json!([]));
}

#[test]
fn reports_a_missing_version_as_a_failure() {
    let fixture = Fixture::new();
    fixture.trust_the_fake_releases();
    let error = fixture.call("install", json!({ "version": "9.9.9", "onProgress": channel(1) })).unwrap_err();
    assert_eq!(error["cancelled"], false);
    assert!(error["message"].as_str().unwrap().contains("isn't in the release list"), "{error}");
}

/// The fake engine's `vox` is a shell script, so this runs where shell scripts do.
#[cfg(unix)]
#[test]
fn creates_and_manages_a_project() {
    let fixture = Fixture::new();
    fixture.trust_the_fake_releases();
    fixture.ok("install", json!({ "version": "0.2.0", "onProgress": channel(1) }));

    let offer = fixture.ok("describe_new", json!({ "version": "0.2.0" }));
    assert_eq!(offer["templates"].as_array().unwrap().len(), 6);
    assert_eq!(offer["templates"][1]["id"], "3d");
    assert!(offer["setup"].as_array().unwrap().iter().any(|item| item["id"] == "sample-scene"));

    assert_eq!(fixture.ok("check_new_project", json!({ "form": fixture.form("Orbit") })), Value::Null);
    let problem = fixture.ok("check_new_project", json!({ "form": fixture.form("Or/bit") }));
    assert!(problem.as_str().unwrap().contains("can't contain"), "{problem}");

    let path = fixture
        .ok("create_project", json!({ "version": "0.2.0", "form": fixture.form("Orbit"), "onStep": channel(2) }));
    let path = path.as_str().unwrap().to_owned();
    assert!(path.ends_with("Orbit"));
    let settings = fixture.ok("get_settings", json!({}));
    assert!(settings["project_location"].as_str().unwrap().ends_with("games"), "the location is remembered");

    let projects = fixture.ok("list_projects", json!({}));
    assert_eq!(projects[0]["name"], "Orbit");
    assert_eq!(projects[0]["engine"], "0.2.0");
    assert_eq!(projects[0]["status"], json!({ "kind": "ready" }));
    assert_eq!(projects[0]["favourite"], false);

    fixture.ok("set_favourite", json!({ "path": path, "favourite": true }));
    assert_eq!(fixture.ok("list_projects", json!({}))[0]["favourite"], true);
    assert_eq!(fixture.ok("projects_using", json!({ "version": "0.2.0" })), json!(["Orbit"]));

    // The editor doesn't exist yet, so opening explains that instead of failing silently.
    let error = fixture.call("open_project", json!({ "path": path })).unwrap_err();
    assert!(error["message"].as_str().unwrap().contains("has no editor"), "{error}");

    fixture.ok("uninstall", json!({ "version": "0.2.0" }));
    let projects = fixture.ok("list_projects", json!({}));
    assert_eq!(projects[0]["status"], json!({ "kind": "engineMissing", "version": "0.2.0" }));

    fixture.ok("remove_project", json!({ "path": path }));
    assert_eq!(fixture.ok("list_projects", json!({})), json!([]));
    assert!(std::path::Path::new(&path).join("voxear.toml").is_file(), "removing never deletes files");
}

/// Upgrades through IPC: the choices, the upgrade with its backup, and restoring the backup.
#[cfg(unix)]
#[test]
fn upgrades_a_project_and_restores_its_backup() {
    let fixture = Fixture::new();
    fixture.trust_the_fake_releases();
    fixture.ok("install", json!({ "version": "0.1.0", "onProgress": channel(1) }));
    fixture.ok("install", json!({ "version": "0.2.0", "onProgress": channel(1) }));
    let path = fixture
        .ok("create_project", json!({ "version": "0.1.0", "form": fixture.form("Orbit"), "onStep": channel(2) }));

    let choices = fixture.ok("upgrade_choices", json!({ "path": path }));
    assert_eq!(choices["current"], "0.1.0");
    let directions: Vec<_> = choices["choices"].as_array().unwrap().iter().map(|c| c["direction"].clone()).collect();
    assert_eq!(directions, [json!("upgrade"), json!("current")]);

    let report = fixture.ok(
        "upgrade_project",
        json!({ "path": path, "version": "0.2.0", "allowDowngrade": false, "onEvent": channel(3) }),
    );
    assert_eq!(report["from"], "0.1.0");
    assert_eq!(report["to"], "0.2.0");
    assert_eq!(report["migrated"], true);
    assert_eq!(report["changes"].as_array().unwrap().len(), 2);
    assert!(report["backup"]["size"].as_u64().unwrap() > 0);
    assert_eq!(fixture.ok("list_projects", json!({}))[0]["engine"], "0.2.0");

    let error = fixture
        .call(
            "upgrade_project",
            json!({ "path": path, "version": "0.1.0", "allowDowngrade": false, "onEvent": channel(3) }),
        )
        .unwrap_err();
    assert!(error["message"].as_str().unwrap().contains("has to be asked for"), "{error}");

    let backups = fixture.ok("list_backups", json!({ "path": path }));
    assert_eq!(backups.as_array().unwrap().len(), 1);
    assert_eq!(backups[0]["changes"], report["changes"]);
    let archive = backups[0]["archive"].clone();

    let restored = fixture.ok("restore_backup", json!({ "archive": archive }));
    assert!(restored.as_str().unwrap().ends_with("Orbit (before 0.2.0)"), "{restored}");
    assert_eq!(fixture.ok("list_projects", json!({})).as_array().unwrap().len(), 2, "the copy is listed");

    fixture.ok("delete_backup", json!({ "archive": archive }));
    assert_eq!(fixture.ok("list_backups", json!({ "path": null })), json!([]));
    assert_eq!(
        fixture.ok("release_notes", json!({ "from": "0.1.0", "to": "0.2.0" })),
        json!([]),
        "the fakes have no notes"
    );
}

#[test]
fn says_why_a_development_build_cannot_update_itself() {
    let fixture = Fixture::new();
    let status = fixture.ok("check_launcher_update", json!({}));
    assert_eq!(status["kind"], "notSetUp");
    assert!(status["reason"].as_str().unwrap().len() > 10);
    let error = fixture.call("install_launcher_update", json!({ "onProgress": channel(4) })).unwrap_err();
    assert!(error["message"].as_str().unwrap().contains("check for updates first"), "{error}");
}

/// The manual as an optional part of an install, through IPC: offered with its size, left out when asked, added
/// later, listed, and removed.
#[test]
fn installs_adds_and_removes_the_manual() {
    let mut fixture = Fixture::new();
    fixture.fake.add_manual("0.2.0");
    fixture.fake.write();
    fixture.trust_the_fake_releases();

    let releases = fixture.ok("list_releases", json!({}));
    let newest = &releases["releases"][0];
    assert_eq!(newest["version"], "0.2.0");
    assert_eq!(newest["modules"][0]["name"], "manual");
    assert_eq!(newest["modules"][0]["title"], "Voxear Manual");
    assert!(newest["modules"][0]["size"].as_u64().unwrap() > 0);
    assert_eq!(releases["releases"][1]["modules"], json!([]), "0.1.0 has no modules");

    fixture.ok("install", json!({ "version": "0.2.0", "without": ["manual"], "onProgress": channel(1) }));
    let manual = |fixture: &Fixture| fixture.ok("list_installs", json!({}))[0]["modules"][0].clone();
    assert_eq!(
        manual(&fixture),
        json!({ "name": "manual", "title": "Voxear Manual", "installed": false, "available": true })
    );

    fixture.ok("add_module", json!({ "version": "0.2.0", "name": "manual", "onProgress": channel(2) }));
    assert_eq!(manual(&fixture)["installed"], true);

    fixture.ok("remove_module", json!({ "version": "0.2.0", "name": "manual" }));
    assert_eq!(manual(&fixture)["installed"], false);

    fixture.ok("uninstall", json!({ "version": "0.2.0" }));
    let install = fixture.ok("install", json!({ "version": "0.2.0", "without": [], "onProgress": channel(1) }));
    assert_eq!(install["version"], "0.2.0");
    assert_eq!(manual(&fixture)["installed"], true, "installed with the engine by default");
}
