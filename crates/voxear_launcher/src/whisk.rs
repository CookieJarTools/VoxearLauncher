//! Whisk, the code editor Voxear opens scripts and projects in. It isn't part of an engine version: one copy
//! serves every engine version, and it keeps itself up to date, so the launcher only installs it, finds it,
//! starts it and removes the copy it installed.
//!
//! Whisk is published on its own GitHub releases, with a manifest in the static JSON format Tauri's updater
//! reads (the same format as the launcher's own updates). Every download is checked against Whisk's update key
//! before anything is installed.
//!
//! - **Linux:** the AppImage goes to `<root>/tools/whisk/Whisk.AppImage`. Whisk replaces it in place when it
//!   updates.
//! - **Windows:** Whisk's per-user installer runs silently, and Whisk lands in `%LOCALAPPDATA%\Whisk`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use semver::Version;

use crate::fsutil;
use crate::layout::Layout;
use crate::platform::Platform;
use crate::self_update;
use crate::store::Progress;

/// Where Whisk's newest release is described.
pub const UPDATES: &str = "https://github.com/CookieJarTools/Whisk/releases/latest/download/latest.json";
/// Overrides where Whisk's release manifest is read from, for mirrors and testing: a URL or a file.
pub const UPDATES_VAR: &str = "VOXUP_WHISK_UPDATES";

/// The key Whisk's releases are signed with, as Whisk's own updater trusts it (base64 of the minisign `.pub`
/// file, from Whisk's `tauri.conf.json`).
pub const WHISK_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDhGM0Y0NEY5ODE1NkY3MDIKUldRQzkxYUIrVVEvanhUSFYzYW4zVXJFelRUd1FYcEx5Zk9SUkEyTHBJamtFRkkrV1F3cE1UK0EK";

/// Where Whisk installs itself on Linux from its `.deb` and `.rpm` packages.
const LINUX_PACKAGE: &str = "/usr/bin/whisk";
/// The registry key Whisk's Windows installer records its folder under.
const WINDOWS_KEY: &str = r"HKCU\Software\CookieJarTools\Whisk";
const WINDOWS_EXE: &str = "whisk.exe";

/// Where Whisk's release manifest is: `VOXUP_WHISK_UPDATES`, or Whisk's GitHub releases.
pub fn manifest_url() -> String {
    std::env::var(UPDATES_VAR).ok().filter(|url| !url.trim().is_empty()).unwrap_or_else(|| UPDATES.to_owned())
}

/// How a Whisk that was found got there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiskSource {
    /// The launcher installed it, and can remove it.
    Launcher,
    /// Whisk's own installer, a package or a copy the user keeps elsewhere.
    Elsewhere,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Whisk {
    /// The program to start.
    pub path: PathBuf,
    pub source: WhiskSource,
}

/// The launcher's own copy of Whisk on Linux.
pub fn launcher_copy(layout: &Layout) -> PathBuf {
    layout.tools().join("whisk").join("Whisk.AppImage")
}

/// Finds Whisk: the launcher's copy first, then wherever Whisk records it is installed, then where its
/// packages and installer put it.
pub fn find(layout: &Layout) -> Option<Whisk> {
    let own = launcher_copy(layout);
    if cfg!(not(windows)) && own.is_file() {
        return Some(Whisk { path: own, source: WhiskSource::Launcher });
    }
    let elsewhere = if cfg!(windows) { find_on_windows() } else { recorded().or_else(|| existing(LINUX_PACKAGE)) };
    elsewhere.map(|path| Whisk { path, source: WhiskSource::Elsewhere })
}

/// Whisk writes where it is to `~/.whisk/install-path` each time it starts on Linux, as an AppImage can live
/// anywhere.
fn record_file() -> Option<PathBuf> {
    // Tests leave the real record alone.
    if cfg!(test) {
        return None;
    }
    dirs::home_dir().map(|home| home.join(".whisk").join("install-path"))
}

/// The recorded Whisk, or, when that file has gone, another Whisk AppImage in its folder: an AppImage updated
/// by something other than Whisk itself can come back under its new version's name.
fn recorded() -> Option<PathBuf> {
    let text = fs::read_to_string(record_file()?).ok()?;
    let path = PathBuf::from(text.trim());
    if path.is_file() {
        return Some(path);
    }
    let mut siblings: Vec<PathBuf> = fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| is_whisk_appimage(path) && path.is_file())
        .collect();
    siblings.sort();
    siblings.pop()
}

/// `Whisk_0.3.2_amd64.AppImage`, or `Whisk.AppImage` as the launcher and Whisk's guide name it.
fn is_whisk_appimage(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.starts_with("whisk") && name.ends_with(".appimage")
}

fn existing(path: impl Into<PathBuf>) -> Option<PathBuf> {
    let path = path.into();
    (!path.as_os_str().is_empty() && path.is_file()).then_some(path)
}

fn find_on_windows() -> Option<PathBuf> {
    let from_registry = Command::new("reg").args(["query", WINDOWS_KEY, "/ve"]).output().ok().and_then(|out| {
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // `    (Default)    REG_SZ    C:\Users\me\AppData\Local\Whisk`
        let folder =
            text.lines().find_map(|line| line.split_once("REG_SZ").map(|(_, folder)| folder.trim().to_owned()))?;
        existing(Path::new(folder.trim_matches('"')).join(WINDOWS_EXE))
    });
    from_registry.or_else(|| existing(dirs::data_local_dir()?.join("Whisk").join(WINDOWS_EXE)))
}

/// What installing Whisk did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    pub whisk: Whisk,
}

/// Installs the newest Whisk from the manifest at `url`, checked against `key` ([`WHISK_KEY`] for Whisk's own
/// releases). Whisk installed elsewhere is left as it is.
pub fn install(
    layout: &Layout,
    url: &str,
    key: &str,
    platform: Platform,
    progress: &mut dyn FnMut(Progress),
) -> Result<Installed> {
    let manifest = self_update::fetch_manifest(url)
        .context("couldn't read Whisk's release list")?
        .ok_or_else(|| anyhow!("no Whisk release is published at {url}"))?;
    let entry = manifest
        .platforms
        .get(platform.as_str())
        .ok_or_else(|| anyhow!("Whisk {} has no download for {platform}", manifest.version))?;

    let downloads = layout.downloads();
    fs::create_dir_all(&downloads).with_context(|| format!("couldn't create {}", downloads.display()))?;
    let name = format!("whisk-{}-{}{}", manifest.version, fsutil::unique_suffix(), platform.exe_suffix());
    let download = downloads.join(name);
    let result = (|| {
        self_update::download_to(&fetch_url(url, &entry.url), &download, &mut |done, total| {
            progress(Progress::Downloading { done, total: total.unwrap_or(0) })
        })?;
        progress(Progress::Verifying);
        self_update::verify_artifact(&download, &entry.signature, &manifest.version, key)
            .context("the Whisk download failed its check, so nothing was installed")?;
        progress(Progress::Finishing);
        match platform {
            Platform::LinuxX86_64 => put_in_place(layout, &download),
            Platform::WindowsX86_64 => run_installer(&download),
        }
    })();
    _ = fs::remove_file(&download);
    let whisk = result?;
    Ok(Installed { version: manifest.version, whisk })
}

fn fetch_url(manifest: &str, url: &str) -> String {
    crate::fetch::resolve(manifest, url)
}

fn put_in_place(layout: &Layout, download: &Path) -> Result<Whisk> {
    let dest = launcher_copy(layout);
    let folder = dest.parent().expect("the copy is inside a folder");
    fs::create_dir_all(folder).with_context(|| format!("couldn't create {}", folder.display()))?;
    self_update::make_executable(download)?;
    fs::rename(download, &dest).with_context(|| format!("couldn't put Whisk at {}", dest.display()))?;
    record_if_unknown(&dest);
    Ok(Whisk { path: dest, source: WhiskSource::Launcher })
}

/// Tells other programs where Whisk is before it has started for the first time, unless they already know of
/// a copy that is still there. Whisk rewrites the file itself each time it starts.
fn record_if_unknown(whisk: &Path) {
    if recorded().is_some() {
        return;
    }
    if let Some(file) = record_file() {
        _ = fsutil::write_atomic(&file, whisk.to_string_lossy().as_bytes());
    }
}

/// Runs Whisk's per-user installer without its pages, then finds where it put Whisk.
fn run_installer(setup: &Path) -> Result<Whisk> {
    let status = Command::new(setup).arg("/S").status().context("couldn't start Whisk's installer")?;
    if !status.success() {
        bail!("Whisk's installer stopped with {status}");
    }
    let path = find_on_windows().ok_or_else(|| anyhow!("Whisk's installer finished, but Whisk can't be found"))?;
    Ok(Whisk { path, source: WhiskSource::Elsewhere })
}

/// Removes the Whisk the launcher installed. Returns whether there was one. A Whisk installed some other way is
/// left alone; on Windows, Whisk is removed from the system's list of installed apps.
pub fn remove(layout: &Layout) -> Result<bool> {
    let own = launcher_copy(layout);
    if !own.is_file() {
        return Ok(false);
    }
    fs::remove_file(&own).with_context(|| format!("couldn't remove {}", own.display()))?;
    if let Some(file) = record_file()
        && fs::read_to_string(&file).is_ok_and(|text| fsutil::same_path(Path::new(text.trim()), &own))
    {
        _ = fs::remove_file(file);
    }
    _ = fs::remove_dir(own.parent().expect("the copy is inside a folder"));
    Ok(true)
}

/// Starts Whisk on `folder`, and on each file in `files` at its line when one is given: Whisk takes
/// `whisk <folder> <file>[:<line>[:<column>]]`. When Whisk is already open, the new process hands them to the
/// open window and quits.
pub fn open(whisk: &Path, folder: Option<&Path>, files: &[(&Path, Option<u32>)]) -> Result<()> {
    let mut command = Command::new(whisk);
    command.args(arguments(folder, files));
    outside_appimage(&mut command);
    let mut child = command.spawn().with_context(|| format!("couldn't start Whisk ({})", whisk.display()))?;
    // Waited on elsewhere, so it doesn't linger once it exits.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The arguments Whisk takes for a folder and files at lines.
pub fn arguments(folder: Option<&Path>, files: &[(&Path, Option<u32>)]) -> Vec<OsString> {
    let mut args: Vec<OsString> = folder.map(|folder| folder.as_os_str().to_owned()).into_iter().collect();
    for (file, line) in files {
        let mut arg = file.as_os_str().to_owned();
        if let Some(line) = line {
            arg.push(format!(":{}", line.max(&1)));
        }
        args.push(arg);
    }
    args
}

/// Started from the launcher's own AppImage, Whisk would inherit the variables that point at the libraries
/// bundled in it, and those are the wrong ones for Whisk. This puts back what the user started the launcher
/// with.
fn outside_appimage(command: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR") else { return };
    let appdir = appdir.to_string_lossy().trim_end_matches('/').to_owned();
    for (key, value) in std::env::vars_os() {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else { continue };
        if ["APPDIR", "APPIMAGE", "ARGV0", "OWD"].contains(&key)
            || (key == "GTK_THEME" && value.starts_with("Adwaita:"))
        {
            command.env_remove(key);
        } else if value.contains(&appdir) {
            let kept: Vec<&str> =
                value.split(':').filter(|part| !part.is_empty() && !part.starts_with(&appdir)).collect();
            match kept.is_empty() {
                true => command.env_remove(key),
                false => command.env(key, kept.join(":")),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    use crate::self_update::{PlatformUpdate, UpdateManifest};
    use crate::testing::TestKey;

    struct Fixture {
        dir: tempfile::TempDir,
        key: TestKey,
    }

    impl Fixture {
        fn new() -> Self {
            Self { dir: tempfile::tempdir().unwrap(), key: TestKey::generate() }
        }

        fn layout(&self) -> Layout {
            Layout::new(self.dir.path().join("voxear"))
        }

        /// A Whisk release as Whisk publishes it: its download and a manifest naming it.
        fn publish(&self, version: &str, signed_as: &str, binary: &[u8]) -> String {
            let site = self.dir.path().join("site");
            fs::create_dir_all(&site).unwrap();
            fs::write(site.join("Whisk.AppImage"), binary).unwrap();
            let comment = format!("timestamp:1793392749\tfile:Whisk_{signed_as}_amd64.AppImage\tversion:{signed_as}");
            let signature = STANDARD.encode(self.key.sign_with_comment(binary, &comment));
            let mut platforms = BTreeMap::new();
            platforms.insert(
                "linux-x86_64".to_owned(),
                PlatformUpdate { signature, url: "Whisk.AppImage".to_owned(), sha256: None, size: None },
            );
            let manifest =
                UpdateManifest { version: Version::parse(version).unwrap(), notes: None, pub_date: None, platforms };
            let path = site.join("latest.json");
            fs::write(&path, manifest.to_json().unwrap()).unwrap();
            path.to_string_lossy().into_owned()
        }

        fn key(&self) -> String {
            STANDARD.encode(self.key.public_file())
        }
    }

    #[test]
    fn trusts_whisks_own_key() {
        let key = self_update::unwrap_base64(WHISK_KEY).unwrap();
        crate::signing::TrustedKeys::new().add(&key).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn installs_a_signed_whisk_and_removes_it() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.3.2", "0.3.2", b"whisk");
        let layout = fixture.layout();
        let mut steps = Vec::new();
        let installed =
            install(&layout, &url, &fixture.key(), Platform::LinuxX86_64, &mut |step| steps.push(step)).unwrap();
        assert_eq!(installed.version, Version::new(0, 3, 2));
        assert_eq!(installed.whisk, Whisk { path: launcher_copy(&layout), source: WhiskSource::Launcher });
        assert_eq!(fs::read(launcher_copy(&layout)).unwrap(), b"whisk");
        assert!(steps.contains(&Progress::Verifying), "{steps:?}");
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(launcher_copy(&layout)).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert_eq!(find(&layout).map(|whisk| whisk.source), Some(WhiskSource::Launcher));
        assert_eq!(fs::read_dir(layout.downloads()).unwrap().count(), 0, "the download is cleaned up");

        assert!(remove(&layout).unwrap());
        assert!(!launcher_copy(&layout).exists());
        assert!(!remove(&layout).unwrap(), "nothing left to remove");
    }

    #[test]
    fn refuses_a_whisk_signed_by_someone_else() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.3.2", "0.3.2", b"whisk");
        let stranger = STANDARD.encode(TestKey::generate().public_file());
        let layout = fixture.layout();
        let error = install(&layout, &url, &stranger, Platform::LinuxX86_64, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("failed its check"), "{error:#}");
        assert!(!launcher_copy(&layout).exists());
    }

    #[test]
    fn refuses_an_old_whisk_offered_as_a_new_one() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.9.0", "0.3.0", b"old whisk");
        let error = install(&fixture.layout(), &url, &fixture.key(), Platform::LinuxX86_64, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("signed as 0.3.0"), "{error:#}");
    }

    #[test]
    fn says_when_nothing_is_published() {
        let fixture = Fixture::new();
        let nowhere = fixture.dir.path().join("nothing").join("latest.json");
        let error =
            install(&fixture.layout(), &nowhere.to_string_lossy(), &fixture.key(), Platform::LinuxX86_64, &mut |_| {})
                .unwrap_err();
        assert!(format!("{error:#}").contains("no Whisk release"), "{error:#}");
    }

    #[test]
    fn opens_a_folder_and_files_at_lines() {
        let args = arguments(
            Some(Path::new("/games/Hello")),
            &[(Path::new("/games/Hello/Scripts/Spinner.cs"), Some(12)), (Path::new("/games/Hello/README.md"), None)],
        );
        assert_eq!(args, ["/games/Hello", "/games/Hello/Scripts/Spinner.cs:12", "/games/Hello/README.md"]);
        assert_eq!(arguments(None, &[(Path::new("a.cs"), Some(0))]), ["a.cs:1"], "lines start at 1");
    }
}
