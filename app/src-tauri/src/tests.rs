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
    /// The fake releases stand in for the official ones.
    fn new() -> Self {
        Self::listing_official_releases_at(|fake| fake.url())
    }

    fn listing_official_releases_at(official: impl FnOnce(&FakeReleases) -> String) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.1.0", Channel::Stable);
        fake.add("0.2.0", Channel::Stable);
        fake.write();
        let launcher = Launcher::with_layout(Layout::new(dir.path().join("home")), Platform::current().unwrap())
            .with_official_releases(official(&fake));
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
        let settings = json!({ "trusted_keys": [self.fake.key.public()] });
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
    let fixture = Fixture::listing_official_releases_at(|fake| {
        fake.dir.parent().unwrap().join("unpublished").join("releases.toml").display().to_string()
    });
    let releases = fixture.ok("list_releases", json!({}));
    assert_eq!(releases["sources"][0]["name"], "Official");
    assert_eq!(releases["sources"][0]["state"], "unpublished");
    assert_eq!(releases["releases"], json!([]));
    assert_eq!(fixture.ok("list_installs", json!({})), json!([]));
}

/// Someone else's releases, such as a fork's, signed with their own key and listed beside the official ones.
#[test]
fn lists_custom_releases_beside_the_official_ones() {
    let fixture = Fixture::new();
    fixture.trust_the_fake_releases();
    let mut fork = FakeReleases::new(fixture.dir.path().join("fork"));
    fork.add("0.2.0+fork", Channel::Stable);
    fork.write();

    let stranger = voxear_launcher::testing::TestKey::generate().public();
    let wrong = json!({ "address": fork.url(), "name": "Fork", "key": stranger });
    let error = fixture.call("add_custom_releases", wrong).unwrap_err();
    assert!(error["message"].as_str().unwrap().contains("signature"), "{error}");
    assert!(fixture.ok("get_settings", json!({})).get("custom_releases").is_none(), "nothing was added");

    let added =
        fixture.ok("add_custom_releases", json!({ "address": fork.url(), "name": "Fork", "key": fork.key.public() }));
    assert_eq!(added, json!({ "name": "Fork", "url": fork.url(), "releases": 1 }));
    let saved = fixture.ok("get_settings", json!({}));
    assert_eq!(saved["custom_releases"][0]["name"], "Fork");
    assert_eq!(saved["custom_releases"][0]["key"], fork.key.public());

    let releases = fixture.ok("list_releases", json!({}));
    let listed: Vec<_> =
        releases["releases"].as_array().unwrap().iter().map(|r| (r["version"].clone(), r["source"].clone())).collect();
    assert_eq!(
        listed,
        [(json!("0.2.0+fork"), json!("Fork")), (json!("0.2.0"), json!(null)), (json!("0.1.0"), json!(null))]
    );
    let states: Vec<_> =
        releases["sources"].as_array().unwrap().iter().map(|s| (s["name"].clone(), s["state"].clone())).collect();
    assert_eq!(states, [(json!("Official"), json!("listed")), (json!("Fork"), json!("listed"))]);

    // A project pinned to the fork's version finds it without being told where.
    let install = fixture.ok("install", json!({ "version": "0.2.0+fork", "onProgress": channel(1) }));
    assert_eq!(install["version"], "0.2.0+fork");
    assert_eq!(fixture.ok("list_installs", json!({}))[0]["source"], "Fork");
    let official =
        fixture.ok("install", json!({ "version": "stable", "source": "Official", "onProgress": channel(2) }));
    assert_eq!(official["version"], "0.2.0");
    assert_eq!(fixture.ok("list_installs", json!({}))[1]["source"], json!(null));

    fixture.ok("remove_custom_releases", json!({ "name": "fork" }));
    let releases = fixture.ok("list_releases", json!({}));
    assert_eq!(releases["sources"].as_array().unwrap().len(), 1);
    assert_eq!(fixture.ok("list_installs", json!({}))[0]["source"], "fork", "still says where it came from");
}

#[test]
fn moves_the_old_release_setting_over() {
    let fixture = Fixture::new();
    let old = fixture.dir.path().join("old-mirror").join("releases.toml").display().to_string();
    let settings = fixture.dir.path().join("home").join("settings.toml");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, format!("releases = {old:?}\nproject_location = \"/games\"\n")).unwrap();

    let saved = fixture.ok("get_settings", json!({}));
    assert_eq!(saved["custom_releases"], json!([{ "name": "old-mirror", "url": old }]));
    assert!(saved.get("releases").is_none(), "{saved}");
    assert_eq!(saved["project_location"], "/games");
    let releases = fixture.ok("list_releases", json!({}));
    assert_eq!(releases["sources"][1]["name"], "old-mirror");
    assert_eq!(releases["sources"][1]["state"], "unpublished");
}

#[test]
fn hides_nightlies_until_asked_or_one_is_installed() {
    let mut fixture = Fixture::new();
    fixture.fake.add("0.3.0-nightly.20261117", Channel::Nightly);
    fixture.fake.write();
    fixture.trust_the_fake_releases();
    assert_eq!(fixture.ok("list_releases", json!({}))["nightlies"], false);

    fixture.ok("install", json!({ "version": "nightly", "onProgress": channel(1) }));
    assert_eq!(fixture.ok("list_releases", json!({}))["nightlies"], true, "one is installed");

    let mut settings = fixture.ok("get_settings", json!({}));
    settings["nightlies"] = json!(false);
    fixture.ok("save_settings", json!({ "settings": settings }));
    assert_eq!(fixture.ok("list_releases", json!({}))["nightlies"], false, "as set");
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
    assert!(error["message"].as_str().unwrap().contains("isn't in the release lists"), "{error}");
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

/// The Dedicated Server goes through the same calls as the manual, offered for this platform beside it.
#[test]
fn installs_adds_and_removes_the_server() {
    let mut fixture = Fixture::new();
    fixture.fake.add_manual("0.2.0");
    fixture.fake.add_server("0.2.0");
    fixture.fake.write();
    fixture.trust_the_fake_releases();

    let releases = fixture.ok("list_releases", json!({}));
    let modules = &releases["releases"][0]["modules"];
    assert_eq!(modules[1]["name"], "server");
    assert_eq!(modules[1]["title"], "Dedicated Server");
    assert!(modules[1]["size"].as_u64().unwrap() > 0);

    fixture.ok("install", json!({ "version": "0.2.0", "without": ["server"], "onProgress": channel(1) }));
    let server = |fixture: &Fixture| fixture.ok("list_installs", json!({}))[0]["modules"][1].clone();
    assert_eq!(
        server(&fixture),
        json!({ "name": "server", "title": "Dedicated Server", "installed": false, "available": true })
    );

    fixture.ok("add_module", json!({ "version": "0.2.0", "name": "server", "onProgress": channel(2) }));
    assert_eq!(server(&fixture)["installed"], true);
    fixture.ok("remove_module", json!({ "version": "0.2.0", "name": "server" }));
    assert_eq!(server(&fixture)["installed"], false);

    let error = fixture.call("remove_module", json!({ "version": "0.2.0", "name": "bin" })).unwrap_err();
    assert!(error.to_string().contains("bin"), "{error}");
}

/// Whisk is found or not wherever this machine keeps it, so only the launcher's own copy is checked here; the
/// install itself is tested in the core against a release signed with a test key.
#[test]
fn says_where_whisk_is_and_removes_only_its_own_copy() {
    let fixture = Fixture::new();
    let status = fixture.ok("whisk_status", json!({}));
    assert_eq!(status["installed"], status["path"].is_string(), "{status}");
    assert_eq!(status["byLauncher"], false, "this launcher hasn't installed Whisk");

    let own = voxear_launcher::whisk::launcher_copy(&Layout::new(fixture.dir.path().join("home")));
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "whisk").unwrap();
    if cfg!(not(windows)) {
        let status = fixture.ok("whisk_status", json!({}));
        assert_eq!(status["byLauncher"], true, "{status}");
        assert_eq!(status["path"], own.display().to_string());
    }
    let after = fixture.ok("remove_whisk", json!({}));
    assert_eq!(after["byLauncher"], false, "{after}");
    assert!(!own.exists());
}
