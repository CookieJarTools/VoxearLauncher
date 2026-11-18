//! The launcher's own updates, separate from engine versions.
//!
//! Each channel has a manifest in the static JSON format Tauri's updater reads, at
//! `https://cookiejartools.github.io/VoxearReleases/launcher/<channel>.json`. The window updates itself through
//! Tauri's updater. A standalone `voxup` updates through this module, from the `voxup-<platform>` entries in the
//! same file, which Tauri's updater ignores.
//!
//! Signatures are minisign, base64-encoded the way Tauri writes them, and are made with the launcher update key,
//! not the engine release key. Their trusted comment must name the version the manifest announces: the
//! manifest itself isn't signed, so this is what stops an old build being offered as a new one. Each download
//! may also list its size and SHA-256, which are checked first, so a download that was cut short says so.
//!
//! Nothing is replaced until every check has passed, so a failed or cut-off download leaves the installed
//! launcher as it was.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fetch;
use crate::fsutil;
use crate::platform::Platform;
use crate::signing::TrustedKeys;
use crate::store::hex;

/// Where the launcher's update manifests are published.
pub const UPDATES_BASE: &str = "https://cookiejartools.github.io/VoxearReleases/launcher";
/// Overrides where update manifests are read from, for mirrors and testing: a URL or a folder.
pub const UPDATES_VAR: &str = "VOXUP_LAUNCHER_UPDATES";

/// The public key launcher updates are signed with, as `tauri signer generate` prints it (base64 of the
/// minisign `.pub` file). Unset until the key is generated (see "Signing keys" in `docs/DESIGN.md`).
pub const LAUNCHER_UPDATE_KEY: Option<&str> = None;

const MAX_MANIFEST: u64 = 1 << 20;

/// The update key this build trusts: the one above, or one given when it was compiled through
/// `VOXEAR_LAUNCHER_UPDATE_KEY`, for trying the update flow with a key of your own.
pub fn update_key() -> Option<&'static str> {
    LAUNCHER_UPDATE_KEY.or(option_env!("VOXEAR_LAUNCHER_UPDATE_KEY")).filter(|key| !key.trim().is_empty())
}

/// The launcher has no nightly channel: nightly engines are installed by a stable or beta launcher.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "stable",
            UpdateChannel::Beta => "beta",
        }
    }
}

impl fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UpdateChannel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "stable" => Ok(UpdateChannel::Stable),
            "beta" => Ok(UpdateChannel::Beta),
            _ => bail!("`{s}` is not a launcher channel (stable or beta)"),
        }
    }
}

/// A channel's update manifest, in Tauri's static JSON format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<String>,
    /// Keyed by target: `linux-x86_64` and `windows-x86_64` for the window, `voxup-<platform>` for `voxup`.
    pub platforms: BTreeMap<String, PlatformUpdate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlatformUpdate {
    /// The artifact's minisign signature, base64-encoded.
    pub signature: String,
    /// Absolute, or relative to the manifest.
    pub url: String,
    /// The artifact's SHA-256, in lowercase hex. Tauri's updater ignores it; the launcher checks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The artifact's size in bytes, checked like `sha256`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl PlatformUpdate {
    /// Checks a downloaded artifact against the size and SHA-256 the manifest lists, where it lists them.
    pub fn check_bytes(&self, bytes: &[u8]) -> Result<()> {
        self.check(bytes.len() as u64, || Ok(hex(&Sha256::digest(bytes))))
    }

    /// [`PlatformUpdate::check_bytes`] for a file on disk.
    pub fn check_file(&self, path: &Path) -> Result<()> {
        let len = fs::metadata(path).with_context(|| format!("couldn't read {}", path.display()))?.len();
        self.check(len, || {
            let mut reader = BufReader::new(File::open(path)?);
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 256 * 1024];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(hex(&hasher.finalize()))
        })
    }

    fn check(&self, len: u64, digest: impl FnOnce() -> Result<String>) -> Result<()> {
        if let Some(size) = self.size {
            ensure!(
                len == size,
                "the update download is {len} bytes, but the release lists {size}; it was cut short or changed, so it \
                 was thrown away"
            );
        }
        if let Some(expected) = &self.sha256 {
            let actual = digest().context("couldn't check the update download")?;
            ensure!(
                actual.eq_ignore_ascii_case(expected.trim()),
                "the update download's SHA-256 doesn't match the release's, so it was thrown away"
            );
        }
        Ok(())
    }
}

/// Whether `available` should replace `current`. Versions are semver, so a beta of a version comes before
/// the version itself (`0.2.0-beta.1` < `0.2.0`), and build metadata after a `+` doesn't count.
pub fn is_newer(available: &Version, current: &Version) -> bool {
    available.cmp_precedence(current).is_gt()
}

impl UpdateManifest {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("the launcher's update manifest isn't valid")
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)? + "\n")
    }
}

/// Where a channel's manifest is.
pub fn manifest_url(channel: UpdateChannel) -> String {
    let base = std::env::var(UPDATES_VAR).ok().filter(|base| !base.trim().is_empty());
    let base = base.as_deref().unwrap_or(UPDATES_BASE);
    let file = format!("{channel}.json");
    if base.contains("://") {
        format!("{}/{file}", base.trim_end_matches('/'))
    } else {
        Path::new(base).join(file).to_string_lossy().into_owned()
    }
}

/// The entry standalone `voxup` binaries are listed under.
pub fn voxup_target(platform: Platform) -> String {
    format!("voxup-{platform}")
}

/// A channel's manifest, or `None` when nothing has been published there yet.
pub fn fetch_manifest(url: &str) -> Result<Option<UpdateManifest>> {
    match fetch::read(url, MAX_MANIFEST) {
        Ok(bytes) => Ok(Some(UpdateManifest::parse(std::str::from_utf8(&bytes).context("it isn't text")?)?)),
        Err(error) if fetch::is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Keys and signatures may be base64-wrapped, as Tauri writes them, or plain minisign text.
pub fn unwrap_base64(text: &str) -> Result<String> {
    let text = text.trim();
    if text.starts_with("untrusted comment:") {
        return Ok(text.to_owned());
    }
    let bytes = STANDARD.decode(text).map_err(|_| anyhow!("it's neither minisign text nor base64"))?;
    String::from_utf8(bytes).map_err(|_| anyhow!("it decodes to something that isn't minisign text"))
}

/// Checks a downloaded update: its signature, against `key`, and that it was signed as the version the manifest
/// announced.
pub fn verify_artifact(path: &Path, signature: &str, announced: &Version, key: &str) -> Result<()> {
    let mut keys = TrustedKeys::new();
    keys.add(&unwrap_base64(key).context("the launcher update key is malformed")?)?;
    let signature = unwrap_base64(signature).context("the update's signature is malformed")?;
    keys.verify_file(path, &signature).context("the update failed its signature check")?;
    // Only read now: the global signature that was just checked is what covers the trusted comment.
    let signed =
        signed_version(&signature).ok_or_else(|| anyhow!("the update's signature doesn't say which version it is"))?;
    ensure!(
        &signed == announced,
        "the update claims to be {announced}, but it was signed as {signed}, so it was thrown away"
    );
    Ok(())
}

/// The `version:` field of a signature's trusted comment, which is tab-separated `key:value` pairs, as the Tauri
/// CLI writes it: `timestamp:1793380000\tfile:voxup\tversion:0.2.0`.
pub fn signed_version(signature: &str) -> Option<Version> {
    let comment = signature.lines().find_map(|line| line.strip_prefix("trusted comment: "))?;
    let version = comment.split('\t').find_map(|field| field.strip_prefix("version:"))?;
    Version::parse(version.trim().trim_start_matches('v')).ok()
}

/// How a `voxup` update went.
#[derive(Clone, Debug, PartialEq)]
pub enum VoxupUpdate {
    /// Nothing is published in that channel yet.
    NothingPublished,
    UpToDate(Version),
    Updated {
        from: Version,
        to: Version,
    },
}

/// Updates the standalone `voxup` at `exe` to the newest version in the channel whose manifest is at `url`.
pub fn update_voxup(
    url: &str,
    key: &str,
    current: &Version,
    exe: &Path,
    platform: Platform,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<VoxupUpdate> {
    let Some(manifest) = fetch_manifest(url)? else { return Ok(VoxupUpdate::NothingPublished) };
    if !is_newer(&manifest.version, current) {
        return Ok(VoxupUpdate::UpToDate(current.clone()));
    }
    let target = voxup_target(platform);
    let entry = manifest
        .platforms
        .get(&target)
        .ok_or_else(|| anyhow!("Voxear Launcher {} has no voxup for {platform} yet", manifest.version))?;

    let download = fsutil::unique_sibling(exe, "update");
    let result = (|| {
        download_to(&fetch::resolve(url, &entry.url), &download, progress)?;
        entry.check_file(&download)?;
        verify_artifact(&download, &entry.signature, &manifest.version, key)?;
        make_executable(&download)?;
        replace_exe(exe, &download)
    })();
    _ = fs::remove_file(&download);
    result?;
    Ok(VoxupUpdate::Updated { from: current.clone(), to: manifest.version })
}

pub(crate) fn download_to(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<()> {
    let fetched = fetch::open(url)?;
    let mut reader = fetched.reader;
    let mut file = File::create(dest).with_context(|| format!("couldn't write {}", dest.display()))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let read = reader.read(&mut buffer).context("the download stopped")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        done += read as u64;
        progress(done, fetched.len);
    }
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).context("couldn't make the update executable")
}

#[cfg(not(unix))]
pub(crate) fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Puts `new` where `exe` is. Windows won't overwrite a running program but will rename it, so the old one is
/// moved aside first and removed the next time `voxup` runs.
pub fn replace_exe(exe: &Path, new: &Path) -> Result<()> {
    let denied = |error: std::io::Error| {
        anyhow!(
            "couldn't replace {} ({error}); if your package manager installed it, update it there instead",
            exe.display()
        )
    };
    if cfg!(windows) {
        let old = old_exe(exe);
        _ = fs::remove_file(&old);
        fs::rename(exe, &old).map_err(denied)?;
        if let Err(error) = fs::rename(new, exe) {
            _ = fs::rename(&old, exe);
            return Err(denied(error));
        }
        return Ok(());
    }
    fs::rename(new, exe).map_err(denied)
}

/// Removes what the last update on Windows moved aside.
pub fn clean_up_after_update(exe: &Path) {
    _ = fs::remove_file(old_exe(exe));
}

fn old_exe(exe: &Path) -> std::path::PathBuf {
    exe.with_extension("old.exe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestKey;

    struct Fixture {
        dir: tempfile::TempDir,
        key: TestKey,
    }

    impl Fixture {
        fn new() -> Self {
            Self { dir: tempfile::tempdir().unwrap(), key: TestKey::generate() }
        }

        /// A published `voxup` and a manifest listing it, as the release tool would make them.
        fn publish(&self, version: &str, signed_as: &str, binary: &[u8]) -> String {
            let site = self.dir.path().join("site");
            fs::create_dir_all(&site).unwrap();
            let platform = Platform::current().unwrap();
            fs::write(site.join("voxup-new"), binary).unwrap();
            let comment = format!("timestamp:1793380000\tfile:voxup\tversion:{signed_as}");
            let signature = STANDARD.encode(self.key.sign_with_comment(binary, &comment));
            let mut platforms = BTreeMap::new();
            let entry = PlatformUpdate {
                signature,
                url: "voxup-new".to_owned(),
                sha256: Some(hex(&Sha256::digest(binary))),
                size: Some(binary.len() as u64),
            };
            platforms.insert(voxup_target(platform), entry);
            platforms.insert(
                "linux-x86_64".to_owned(),
                PlatformUpdate {
                    signature: "for the window".to_owned(),
                    url: "Launcher.AppImage".to_owned(),
                    sha256: None,
                    size: None,
                },
            );
            let manifest = UpdateManifest {
                version: Version::parse(version).unwrap(),
                notes: Some("Faster installs".to_owned()),
                pub_date: Some("2026-11-15T12:00:00Z".to_owned()),
                platforms,
            };
            let path = site.join("stable.json");
            fs::write(&path, manifest.to_json().unwrap()).unwrap();
            path.to_string_lossy().into_owned()
        }

        fn installed_voxup(&self) -> std::path::PathBuf {
            let exe = self.dir.path().join("bin").join("voxup");
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, "old voxup").unwrap();
            exe
        }

        fn tauri_key(&self) -> String {
            STANDARD.encode(self.key.public_file())
        }

        fn update(&self, url: &str, exe: &Path) -> Result<VoxupUpdate> {
            let platform = Platform::current().unwrap();
            update_voxup(url, &self.tauri_key(), &Version::new(0, 1, 0), exe, platform, &mut |_, _| {})
        }
    }

    #[test]
    fn reads_tauris_manifest_format() {
        let text = r#"{
            "version": "0.2.0",
            "notes": "What changed",
            "pub_date": "2026-11-15T12:00:00Z",
            "platforms": {
                "linux-x86_64": { "signature": "c2ln", "url": "https://example.invalid/VoxearLauncher_0.2.0_amd64.AppImage" },
                "windows-x86_64": { "signature": "c2ln", "url": "https://example.invalid/VoxearLauncher_0.2.0_x64-setup.exe" }
            }
        }"#;
        let manifest = UpdateManifest::parse(text).unwrap();
        assert_eq!(manifest.version, Version::new(0, 2, 0));
        assert_eq!(manifest.platforms.len(), 2);
        assert_eq!(manifest.platforms["linux-x86_64"].sha256, None);
        assert_eq!(UpdateManifest::parse(&manifest.to_json().unwrap()).unwrap(), manifest);
    }

    #[test]
    fn reads_sizes_and_hashes_beside_tauris_fields() {
        let text = r#"{
            "version": "0.2.0",
            "platforms": {
                "linux-x86_64": {
                    "signature": "c2ln",
                    "url": "https://example.invalid/VoxearLauncher_0.2.0_amd64.AppImage",
                    "sha256": "ab12",
                    "size": 1234
                }
            }
        }"#;
        let manifest = UpdateManifest::parse(text).unwrap();
        let entry = &manifest.platforms["linux-x86_64"];
        assert_eq!((entry.sha256.as_deref(), entry.size), (Some("ab12"), Some(1234)));
        let written = manifest.to_json().unwrap();
        assert!(written.contains("\"sha256\": \"ab12\""), "{written}");
        assert!(!written.contains("notes"), "fields that aren't set are left out: {written}");
    }

    #[test]
    fn refuses_a_manifest_that_isnt_one() {
        assert!(UpdateManifest::parse("{}").is_err());
        assert!(UpdateManifest::parse(r#"{"version": "two", "platforms": {}}"#).is_err());
        assert!(UpdateManifest::parse("<html>Not found</html>").is_err());
    }

    #[test]
    fn updates_voxup_to_a_newer_signed_version() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.2.0", "0.2.0", b"new voxup");
        let exe = fixture.installed_voxup();
        let outcome = fixture.update(&url, &exe).unwrap();
        assert_eq!(outcome, VoxupUpdate::Updated { from: Version::new(0, 1, 0), to: Version::new(0, 2, 0) });
        assert_eq!(fs::read(&exe).unwrap(), b"new voxup");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0o111);
        }
    }

    #[test]
    fn a_cut_off_download_leaves_voxup_as_it_was() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.2.0", "0.2.0", b"new voxup");
        // The file on the server is shorter than the manifest says, as when a download stops part way.
        fs::write(fixture.dir.path().join("site").join("voxup-new"), b"new vo").unwrap();
        let exe = fixture.installed_voxup();
        let error = fixture.update(&url, &exe).unwrap_err();
        assert!(format!("{error:#}").contains("cut short"), "{error:#}");
        assert_eq!(fs::read(&exe).unwrap(), b"old voxup");
        let leftovers = fs::read_dir(exe.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "the partial download was cleaned up");
    }

    #[test]
    fn checks_the_size_and_hash_a_manifest_lists() {
        let entry = PlatformUpdate {
            signature: String::new(),
            url: String::new(),
            sha256: Some(hex(&Sha256::digest(b"launcher 0.2.0"))),
            size: Some(14),
        };
        entry.check_bytes(b"launcher 0.2.0").unwrap();
        let short = entry.check_bytes(b"launcher").unwrap_err();
        assert!(short.to_string().contains("cut short"), "{short}");
        let changed = entry.check_bytes(b"launcher 0.2.X").unwrap_err();
        assert!(changed.to_string().contains("SHA-256"), "{changed}");

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("download");
        fs::write(&file, b"launcher 0.2.0").unwrap();
        entry.check_file(&file).unwrap();
        fs::write(&file, b"launcher 0.2.X").unwrap();
        assert!(entry.check_file(&file).is_err());

        // A manifest from before sizes and hashes were listed is checked by its signature alone.
        let bare = PlatformUpdate { sha256: None, size: None, ..entry };
        bare.check_bytes(b"anything").unwrap();
    }

    #[test]
    fn compares_versions_as_semver() {
        let v = |text: &str| Version::parse(text).unwrap();
        assert!(is_newer(&v("0.2.0"), &v("0.1.0")));
        assert!(is_newer(&v("0.10.0"), &v("0.9.0")), "numbers compare as numbers, not text");
        assert!(is_newer(&v("0.2.0"), &v("0.2.0-beta.3")), "a version comes after its betas");
        assert!(is_newer(&v("0.2.0-beta.10"), &v("0.2.0-beta.9")));
        assert!(!is_newer(&v("0.2.0-beta.1"), &v("0.2.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.2.0")));
        assert!(!is_newer(&v("0.1.9"), &v("0.2.0")), "never offers an older version");
        assert!(!is_newer(&v("0.2.0+linux"), &v("0.2.0")), "build metadata doesn't make a version newer");
    }

    #[test]
    fn refuses_an_old_build_offered_as_a_new_one() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.9.0", "0.2.0", b"an old voxup");
        let exe = fixture.installed_voxup();
        let error = fixture.update(&url, &exe).unwrap_err();
        assert!(format!("{error:#}").contains("signed as 0.2.0"), "{error:#}");
        assert_eq!(fs::read(&exe).unwrap(), b"old voxup", "nothing was replaced");
    }

    #[test]
    fn refuses_an_update_signed_by_someone_else() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.2.0", "0.2.0", b"new voxup");
        let exe = fixture.installed_voxup();
        let stranger = STANDARD.encode(TestKey::generate().public_file());
        let platform = Platform::current().unwrap();
        let error = update_voxup(&url, &stranger, &Version::new(0, 1, 0), &exe, platform, &mut |_, _| {}).unwrap_err();
        assert!(format!("{error:#}").contains("signature"), "{error:#}");
        assert_eq!(fs::read(&exe).unwrap(), b"old voxup");
    }

    #[test]
    fn stays_put_when_up_to_date_or_nothing_is_published() {
        let fixture = Fixture::new();
        let url = fixture.publish("0.1.0", "0.1.0", b"same");
        let exe = fixture.installed_voxup();
        assert_eq!(fixture.update(&url, &exe).unwrap(), VoxupUpdate::UpToDate(Version::new(0, 1, 0)));
        let nowhere = fixture.dir.path().join("nothing").join("stable.json");
        assert_eq!(fixture.update(&nowhere.to_string_lossy(), &exe).unwrap(), VoxupUpdate::NothingPublished);
    }

    #[test]
    fn reads_the_signed_version() {
        let key = TestKey::generate();
        let signature = key.sign_with_comment(b"x", "timestamp:1\tfile:a\tversion:v1.2.3");
        assert_eq!(signed_version(&signature), Some(Version::new(1, 2, 3)));
        assert_eq!(signed_version(&key.sign(b"x")), None);
    }

    #[test]
    fn knows_where_each_channel_is() {
        assert!(UPDATES_BASE.starts_with("https://"));
        assert!(manifest_url(UpdateChannel::Beta).ends_with("beta.json"));
        assert_eq!("stable".parse::<UpdateChannel>().unwrap(), UpdateChannel::Stable);
        assert!("nightly".parse::<UpdateChannel>().is_err());
    }
}
