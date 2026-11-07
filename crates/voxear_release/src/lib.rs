//! Publishing Voxear releases: signing engine archives and launcher builds, and writing the manifests the
//! launcher reads. The output is a site folder laid out the way GitHub Pages serves it for
//! `CookieJarTools/VoxearReleases`:
//!
//! ```text
//! <site>/engine/releases.toml           the engine release manifest
//! <site>/engine/releases.toml.minisig   its signature
//! <site>/launcher/stable.json           the launcher's update manifest per channel
//! <site>/launcher/beta.json
//! ```
//!
//! The archives and builds themselves are uploaded as release assets, next to their `.minisig` files.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use jiff::{SignedDuration, Timestamp};
use semver::Version;
use sha2::{Digest, Sha256};
use voxear_launcher::archive;
use voxear_launcher::engine::{ENGINE_FILE, EngineInfo};
use voxear_launcher::manifest::{self, Download, Module, ModuleDownload, Release, ReleaseManifest, SIGNATURE_SUFFIX};
use voxear_launcher::platform::Platform;
use voxear_launcher::self_update::{self, PlatformUpdate, UpdateChannel, UpdateManifest};
use voxear_launcher::store::hex;

/// Where release assets are downloaded from: `<base>/<tag>/<file>`.
pub const DOWNLOAD_BASE: &str = "https://github.com/CookieJarTools/VoxearReleases/releases/download";
/// Read for key passwords, so CI can sign without a prompt.
pub const PASSWORD_VARS: [&str; 2] = ["VOXEAR_SIGNING_PASSWORD", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"];

/// A signing key, with what it needs to sign.
pub struct Key {
    secret: minisign::SecretKey,
}

/// The files `keygen` wrote, and the public key in the forms the launcher wants.
pub struct NewKey {
    pub secret: PathBuf,
    pub public: PathBuf,
    /// The base64 line, for `ENGINE_RELEASE_KEYS` and `trusted_keys`.
    pub public_line: String,
    /// The whole `.pub` file in base64, as Tauri wants it, for `LAUNCHER_UPDATE_KEY` and `tauri.conf.json`.
    pub public_tauri: String,
}

/// Makes a key pair at `path` (`path.key` and `path.pub`), with the secret key encrypted by `password`; without
/// one, minisign asks for it. With `tauri`, both files are base64-wrapped as `tauri signer generate` writes them,
/// so `tauri build` can use the key.
pub fn keygen(path: &Path, password: Option<String>, tauri: bool) -> Result<NewKey> {
    let secret = path.with_extension("key");
    ensure!(!secret.exists(), "{} already exists; keys are never overwritten", secret.display());
    let pair = minisign::KeyPair::generate_encrypted_keypair(password).context("couldn't make a key")?;
    let secret_text = pair.sk.to_box(Some("Voxear signing key"))?.to_string();
    let public_text = pair.pk.to_box()?.to_string();
    let wrap = |text: &str| if tauri { STANDARD.encode(text) } else { text.to_owned() };

    let public = path.with_extension("pub");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&secret, wrap(&secret_text)).with_context(|| format!("couldn't write {}", secret.display()))?;
    fs::write(&public, wrap(&public_text)).with_context(|| format!("couldn't write {}", public.display()))?;
    Ok(NewKey { secret, public, public_line: pair.pk.to_base64(), public_tauri: STANDARD.encode(&public_text) })
}

impl Key {
    /// Loads a secret key: a minisign `.key` file, or one base64-wrapped as Tauri writes them. `password` unlocks
    /// it; without one, minisign asks for it.
    pub fn load(path: &Path, password: Option<String>) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("couldn't read {}", path.display()))?;
        let text =
            self_update::unwrap_base64(&text).with_context(|| format!("{} isn't a signing key", path.display()))?;
        let boxed = minisign::SecretKeyBox::from_string(&text)
            .map_err(|e| anyhow!("{} isn't a signing key: {e}", path.display()))?;
        let secret = boxed.into_secret_key(password).map_err(|e| anyhow!("couldn't unlock {}: {e}", path.display()))?;
        Ok(Self { secret })
    }

    /// The password from the environment, if CI set one.
    pub fn password_from_env() -> Option<String> {
        PASSWORD_VARS.iter().find_map(|var| std::env::var(var).ok())
    }

    /// Signs `path`, recording the file name and the version it is in the trusted comment, as the Tauri CLI does.
    pub fn sign_file(&self, path: &Path, version: &Version) -> Result<String> {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let comment = format!("timestamp:{}\tfile:{name}\tversion:{version}", Timestamp::now().as_second());
        let file = File::open(path).with_context(|| format!("couldn't read {}", path.display()))?;
        let signature =
            minisign::sign(None, &self.secret, file, Some(&comment), Some("signature from the Voxear release key"))
                .map_err(|e| anyhow!("couldn't sign {}: {e}", path.display()))?;
        Ok(signature.into_string())
    }

    fn sign_bytes(&self, bytes: &[u8]) -> Result<String> {
        let comment = format!("timestamp:{}", Timestamp::now().as_second());
        let signature =
            minisign::sign(None, &self.secret, bytes, Some(&comment), Some("signature from the Voxear release key"))
                .map_err(|e| anyhow!("couldn't sign the manifest: {e}"))?;
        Ok(signature.into_string())
    }
}

/// What to say about an engine release that isn't in its archive.
#[derive(Default)]
pub struct EngineRelease {
    /// The release tag the archive is uploaded under; `engine-v<version>` if not given.
    pub tag: Option<String>,
    pub download_base: Option<String>,
    pub notes: Option<String>,
    pub min_launcher: Option<Version>,
}

/// Adds an engine archive to the site's release manifest: signs the archive (writing `<archive>.minisig` next to
/// it), lists it under its version and platform, and signs the manifest again. The version, channel and platform
/// come from the archive's own `engine.toml`. Returns the release.
pub fn add_engine(site: &Path, archive: &Path, key: &Key, details: &EngineRelease) -> Result<Release> {
    let info = engine_info(archive)?;
    ensure!(
        info.channel.accepts(&info.version),
        "{} says it's {} in the {} channel, which doesn't fit its version",
        archive.display(),
        info.version,
        info.channel
    );
    let signature = key.sign_file(archive, &info.version)?;
    fs::write(signature_path(archive), &signature)?;
    let bytes = fs::read(archive).with_context(|| format!("couldn't read {}", archive.display()))?;
    let file = archive.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let tag = details.tag.clone().unwrap_or_else(|| format!("engine-v{}", info.version));
    let base = details.download_base.as_deref().unwrap_or(DOWNLOAD_BASE).trim_end_matches('/');
    let download = Download {
        platform: info.platform.clone(),
        url: format!("{base}/{tag}/{file}"),
        size: bytes.len() as u64,
        sha256: hex(&Sha256::digest(&bytes)),
        signature,
        modules: Vec::new(),
    };

    let mut releases = load_engine_manifest(site)?;
    let release = match releases.releases.iter_mut().find(|release| release.version == info.version) {
        Some(release) => {
            ensure!(
                release.channel == info.channel,
                "{} is already listed in the {} channel",
                info.version,
                release.channel
            );
            release
        }
        None => {
            releases.releases.push(Release {
                version: info.version.clone(),
                channel: info.channel,
                published: Timestamp::now(),
                notes: None,
                min_launcher: None,
                yanked: false,
                downloads: Vec::new(),
                modules: Vec::new(),
            });
            releases.releases.last_mut().expect("just pushed")
        }
    };
    // A platform's own modules, such as the Dedicated Server, stay listed when its engine archive is added again.
    let modules = match release.downloads.iter().position(|existing| existing.platform == download.platform) {
        Some(index) => release.downloads.remove(index).modules,
        None => Vec::new(),
    };
    release.downloads.push(Download { modules, ..download });
    release.downloads.sort_by(|a, b| a.platform.cmp(&b.platform));
    if details.notes.is_some() {
        release.notes = details.notes.clone();
    }
    if details.min_launcher.is_some() {
        release.min_launcher = details.min_launcher.clone();
    }
    let release = release.clone();
    save_engine_manifest(site, releases, key)?;
    Ok(release)
}

/// What one archive given to `engine add` turned out to be.
pub enum Added {
    Engine(Release),
    Module { file: ModuleFile, module: ModuleDownload },
}

/// A module archive, known by its file name.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleFile {
    pub version: Version,
    pub module: Module,
    /// For a module built per platform, such as the Dedicated Server.
    pub platform: Option<Platform>,
}

/// Adds engine archives and module archives together, as `voxear-dist` writes them: `voxear-<version>-<platform>
/// .tar.zst` with an `engine.toml`, and modules such as `voxear-<version>-manual.tar.zst` and
/// `voxear-<version>-<platform>-server.tar.zst`. Engine archives go first, so a module always finds its release,
/// whatever order the files came in.
pub fn add_archives(site: &Path, archives: &[PathBuf], key: &Key, details: &EngineRelease) -> Result<Vec<Added>> {
    let mut engines = Vec::new();
    let mut modules = Vec::new();
    for archive in archives {
        if engine_info(archive).is_ok() {
            engines.push(archive);
        } else {
            let file = module_of(archive).ok_or_else(|| {
                anyhow!(
                    "{} has no {ENGINE_FILE} and isn't named like a module (voxear-<version>-manual.tar.zst or \
                     voxear-<version>-<platform>-server.tar.zst)",
                    archive.display()
                )
            })?;
            modules.push((archive, file));
        }
    }
    let mut added = Vec::new();
    for archive in engines {
        added.push(Added::Engine(add_engine(site, archive, key, details)?));
    }
    for (archive, file) in modules {
        let module = add_module(site, archive, &file, key, details)?;
        added.push(Added::Module { file, module });
    }
    Ok(added)
}

/// Lists a module archive on an engine release that is already in the manifest: the manual on the release, the
/// Dedicated Server on its platform's download. Every entry must be what the module adds to an install, so it can
/// never change the engine's own files.
pub fn add_module(
    site: &Path,
    archive: &Path,
    file: &ModuleFile,
    key: &Key,
    details: &EngineRelease,
) -> Result<ModuleDownload> {
    let ModuleFile { version, module, platform } = file;
    // The manual's path is the same on every platform.
    archive::check_module(archive, *module, platform.unwrap_or(Platform::ALL[0]))
        .with_context(|| format!("{} can't be listed", archive.display()))?;
    let mut releases = load_engine_manifest(site)?;
    let release = releases.releases.iter_mut().find(|release| &release.version == version).ok_or_else(|| {
        anyhow!(
            "{} is the {module} of Voxear {version}, which isn't in the manifest yet; add its engine archive too",
            archive.display()
        )
    })?;
    let listed = match platform {
        Some(platform) => {
            let download = release.downloads.iter_mut().find(|download| download.platform == platform.as_str());
            let download = download.ok_or_else(|| {
                anyhow!(
                    "{} is the {module} of Voxear {version} for {platform}, which has no engine archive in the \
                     manifest yet; add it too",
                    archive.display()
                )
            })?;
            &mut download.modules
        }
        None => &mut release.modules,
    };

    let signature = key.sign_file(archive, version)?;
    fs::write(signature_path(archive), &signature)?;
    let bytes = fs::read(archive).with_context(|| format!("couldn't read {}", archive.display()))?;
    let name = archive.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let tag = details.tag.clone().unwrap_or_else(|| format!("engine-v{version}"));
    let base = details.download_base.as_deref().unwrap_or(DOWNLOAD_BASE).trim_end_matches('/');
    let entry = ModuleDownload {
        name: *module,
        title: module.title().to_owned(),
        url: format!("{base}/{tag}/{name}"),
        size: bytes.len() as u64,
        sha256: hex(&Sha256::digest(&bytes)),
        signature,
    };
    listed.retain(|existing| existing.name != entry.name);
    listed.push(entry.clone());
    listed.sort_by_key(|existing| existing.name);
    save_engine_manifest(site, releases, key)?;
    Ok(entry)
}

/// What a module archive's file name says it is: `voxear-0.3.0-manual.tar.zst` is the manual of 0.3.0, and
/// `voxear-0.3.0-linux-x86_64-server.tar.zst` its Dedicated Server for Linux. The module is the last part, so
/// versions with tags (`0.4.0-nightly.20261101`) read correctly.
pub fn module_of(archive: &Path) -> Option<ModuleFile> {
    let file = archive.file_name()?.to_str()?;
    let middle = file.strip_prefix("voxear-")?.strip_suffix(".tar.zst")?;
    let (rest, name) = middle.rsplit_once('-')?;
    let module = Module::from_name(name)?;
    if !module.per_platform() {
        return Some(ModuleFile { version: Version::parse(rest).ok()?, module, platform: None });
    }
    Platform::ALL.into_iter().find_map(|platform| {
        let version = rest.strip_suffix(platform.as_str())?.strip_suffix('-')?;
        Some(ModuleFile { version: Version::parse(version).ok()?, module, platform: Some(platform) })
    })
}

/// Marks a release as pulled, or puts it back: it stays installable by exact version, so pinned projects open.
pub fn yank_engine(site: &Path, version: &Version, yanked: bool, key: &Key) -> Result<()> {
    let mut releases = load_engine_manifest(site)?;
    let release = releases
        .releases
        .iter_mut()
        .find(|release| &release.version == version)
        .ok_or_else(|| anyhow!("{version} isn't in the release manifest"))?;
    release.yanked = yanked;
    save_engine_manifest(site, releases, key)
}

/// Removes old nightlies, keeping the newest `keep`.
pub fn prune_nightlies(site: &Path, keep: usize, key: &Key) -> Result<Vec<Version>> {
    let mut releases = load_engine_manifest(site)?;
    let mut nightlies: Vec<Version> = releases
        .releases
        .iter()
        .filter(|release| release.channel == voxear_launcher::Channel::Nightly)
        .map(|release| release.version.clone())
        .collect();
    nightlies.sort();
    let removed: Vec<Version> = nightlies.iter().rev().skip(keep).cloned().collect();
    releases.releases.retain(|release| !removed.contains(&release.version));
    save_engine_manifest(site, releases, key)?;
    Ok(removed)
}

pub fn engine_manifest_path(site: &Path) -> PathBuf {
    site.join("engine").join(manifest::MANIFEST_FILE)
}

fn load_engine_manifest(site: &Path) -> Result<ReleaseManifest> {
    let path = engine_manifest_path(site);
    match fs::read_to_string(&path) {
        Ok(text) => ReleaseManifest::parse(&text).with_context(|| format!("couldn't read {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(ReleaseManifest { format: manifest::FORMAT, generated: Timestamp::now(), releases: Vec::new() })
        }
        Err(e) => Err(e).with_context(|| format!("couldn't read {}", path.display())),
    }
}

/// Writes the manifest with a `generated` time later than the last one, as the launcher requires, and signs it.
fn save_engine_manifest(site: &Path, mut releases: ReleaseManifest, key: &Key) -> Result<()> {
    let path = engine_manifest_path(site);
    let previous =
        fs::read_to_string(&path).ok().and_then(|text| ReleaseManifest::parse(&text).ok()).map(|m| m.generated);
    let now = Timestamp::now();
    releases.generated = match previous {
        Some(previous) if now <= previous => previous.checked_add(SignedDuration::from_secs(1))?,
        _ => now,
    };
    releases.releases.sort_by(|a, b| b.version.cmp(&a.version));
    let text = releases.to_toml()?;
    ReleaseManifest::parse(&text).context("the manifest this would write isn't valid")?;
    fs::create_dir_all(path.parent().expect("the manifest is in a folder"))?;
    fs::write(&path, &text).with_context(|| format!("couldn't write {}", path.display()))?;
    fs::write(signature_path(&path), key.sign_bytes(text.as_bytes())?)?;
    Ok(())
}

/// Reads `engine.toml` from an engine archive.
pub fn engine_info(archive: &Path) -> Result<EngineInfo> {
    let file = File::open(archive).with_context(|| format!("couldn't open {}", archive.display()))?;
    let mut tar = tar::Archive::new(zstd::Decoder::new(file).context("it isn't a .tar.zst")?);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(ENGINE_FILE) {
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            return toml::from_str(&text)
                .with_context(|| format!("the {ENGINE_FILE} in {} isn't valid", archive.display()));
        }
    }
    bail!("{} has no {ENGINE_FILE} at its root, so it isn't an engine release", archive.display())
}

/// What to say about a launcher build.
pub struct LauncherBuild {
    pub channel: UpdateChannel,
    pub version: Version,
    /// `linux-x86_64` or `windows-x86_64` for the window; `voxup-<platform>` for a standalone `voxup`.
    pub target: String,
    pub tag: Option<String>,
    pub download_base: Option<String>,
    pub notes: Option<String>,
}

/// How a launcher build is signed: by Tauri while bundling (its `.sig` file), or here.
pub enum LauncherSignature<'a> {
    TauriSig(&'a Path),
    Sign(&'a Key),
}

/// Lists a launcher build in its channel's update manifest. A new version starts a fresh manifest; the same
/// version adds or replaces that target. Returns where the manifest is.
pub fn add_launcher(site: &Path, file: &Path, build: &LauncherBuild, signature: LauncherSignature) -> Result<PathBuf> {
    let signature = match signature {
        LauncherSignature::TauriSig(sig) => {
            let text = fs::read_to_string(sig).with_context(|| format!("couldn't read {}", sig.display()))?;
            let signed = self_update::signed_version(&self_update::unwrap_base64(&text)?);
            ensure!(
                signed.as_ref() == Some(&build.version),
                "{} was signed for {}, not {}",
                sig.display(),
                signed.map_or_else(|| "no version".to_owned(), |v| v.to_string()),
                build.version
            );
            text.trim().to_owned()
        }
        LauncherSignature::Sign(key) => {
            let text = key.sign_file(file, &build.version)?;
            fs::write(signature_path(file), STANDARD.encode(&text))?;
            STANDARD.encode(text)
        }
    };
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let tag = build.tag.clone().unwrap_or_else(|| format!("launcher-v{}", build.version));
    let base = build.download_base.as_deref().unwrap_or(DOWNLOAD_BASE).trim_end_matches('/');

    let path = site.join("launcher").join(format!("{}.json", build.channel));
    let existing = fs::read_to_string(&path).ok().map(|text| UpdateManifest::parse(&text)).transpose()?;
    let mut manifest = match existing {
        Some(manifest) if manifest.version == build.version => manifest,
        Some(manifest) if manifest.version > build.version => {
            bail!(
                "the {} channel already offers {}, which is newer than {}",
                build.channel,
                manifest.version,
                build.version
            )
        }
        _ => UpdateManifest {
            version: build.version.clone(),
            notes: None,
            pub_date: None,
            platforms: Default::default(),
        },
    };
    manifest.pub_date = Some(Timestamp::now().to_string());
    if build.notes.is_some() {
        manifest.notes = build.notes.clone();
    }
    manifest.platforms.insert(build.target.clone(), PlatformUpdate { signature, url: format!("{base}/{tag}/{name}") });
    fs::create_dir_all(path.parent().expect("the manifest is in a folder"))?;
    fs::write(&path, manifest.to_json()?).with_context(|| format!("couldn't write {}", path.display()))?;
    Ok(path)
}

pub fn signature_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(SIGNATURE_SUFFIX);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use voxear_launcher::self_update::VoxupUpdate;
    use voxear_launcher::testing::engine_archive;
    use voxear_launcher::{Channel, Layout, Platform, Selector, Store, TrustedKeys, releases};

    use super::*;

    struct Fixture {
        dir: tempfile::TempDir,
        site: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let site = dir.path().join("site");
            Self { dir, site }
        }

        fn key(&self, name: &str, tauri: bool) -> (Key, NewKey) {
            let made =
                keygen(&self.dir.path().join("keys").join(name), Some("correct horse".to_owned()), tauri).unwrap();
            (Key::load(&made.secret, Some("correct horse".to_owned())).unwrap(), made)
        }

        fn archive(&self, version: &str, channel: Channel) -> PathBuf {
            let platform = Platform::current().unwrap();
            let path = self.dir.path().join("build").join(format!("voxear-{version}-{platform}.tar.zst"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, engine_archive(&Version::parse(version).unwrap(), channel, platform)).unwrap();
            path
        }

        /// Download URLs pointing at the build folder, standing in for the GitHub release assets.
        fn local(&self) -> EngineRelease {
            EngineRelease {
                download_base: Some(self.dir.path().display().to_string()),
                tag: Some("build".to_owned()),
                ..Default::default()
            }
        }
    }

    #[test]
    fn publishes_engine_releases_the_launcher_installs() {
        let fixture = Fixture::new();
        let (key, made) = fixture.key("engine", false);
        let first =
            add_engine(&fixture.site, &fixture.archive("0.3.0", Channel::Stable), &key, &fixture.local()).unwrap();
        assert_eq!(first.version, Version::new(0, 3, 0));
        add_engine(&fixture.site, &fixture.archive("0.4.0-beta.1", Channel::Beta), &key, &fixture.local()).unwrap();
        assert!(signature_path(&fixture.archive_path_for("0.3.0")).is_file(), "the archive's .minisig is beside it");

        let mut keys = TrustedKeys::new();
        keys.add(&made.public_line).unwrap();
        let layout = Layout::new(fixture.dir.path().join("home"));
        let url = engine_manifest_path(&fixture.site).to_string_lossy().into_owned();
        let loaded = releases::load(&url, &keys, &layout).unwrap();
        assert_eq!(loaded.manifest.releases.len(), 2);

        let store = Store::new(layout, Platform::current().unwrap());
        let release = loaded.manifest.select(&Selector::Latest(Channel::Stable), store.platform()).unwrap();
        let install = store.install(release, &loaded.url, &keys, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(install.version, Version::new(0, 3, 0));
    }

    #[test]
    fn every_publish_moves_the_manifest_forward() {
        let fixture = Fixture::new();
        let (key, _) = fixture.key("engine", false);
        add_engine(&fixture.site, &fixture.archive("0.3.0", Channel::Stable), &key, &fixture.local()).unwrap();
        let read =
            || ReleaseManifest::parse(&fs::read_to_string(engine_manifest_path(&fixture.site)).unwrap()).unwrap();
        let before = read().generated;
        yank_engine(&fixture.site, &Version::new(0, 3, 0), true, &key).unwrap();
        let after = read();
        assert!(after.generated > before);
        assert!(after.releases[0].yanked);
    }

    #[test]
    fn refuses_an_archive_without_engine_toml() {
        let fixture = Fixture::new();
        let (key, _) = fixture.key("engine", false);
        let path = fixture.dir.path().join("not-an-engine.tar.zst");
        fs::write(&path, voxear_launcher::testing::tar_zst(&[])).unwrap();
        let error = add_engine(&fixture.site, &path, &key, &fixture.local()).err().unwrap();
        assert!(error.to_string().contains("isn't an engine release"), "{error}");
    }

    #[test]
    fn keeps_only_the_newest_nightlies() {
        let fixture = Fixture::new();
        let (key, _) = fixture.key("engine", false);
        for day in 1..=4 {
            let version = format!("0.4.0-nightly.2026110{day}");
            add_engine(&fixture.site, &fixture.archive(&version, Channel::Nightly), &key, &fixture.local()).unwrap();
        }
        let removed = prune_nightlies(&fixture.site, 2, &key).unwrap();
        assert_eq!(removed.len(), 2);
        let left = ReleaseManifest::parse(&fs::read_to_string(engine_manifest_path(&fixture.site)).unwrap()).unwrap();
        let versions: Vec<_> = left.releases.iter().map(|r| r.version.to_string()).collect();
        assert_eq!(versions, ["0.4.0-nightly.20261104", "0.4.0-nightly.20261103"]);
    }

    #[test]
    fn publishes_a_voxup_that_updates_itself() {
        let fixture = Fixture::new();
        let (key, made) = fixture.key("launcher", true);
        let platform = Platform::current().unwrap();
        let binary = fixture.dir.path().join("build").join("voxup");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "voxup 0.2.0").unwrap();
        let build = LauncherBuild {
            channel: UpdateChannel::Stable,
            version: Version::new(0, 2, 0),
            target: self_update::voxup_target(platform),
            tag: Some("build".to_owned()),
            download_base: Some(fixture.dir.path().display().to_string()),
            notes: Some("Faster installs".to_owned()),
        };
        let manifest = add_launcher(&fixture.site, &binary, &build, LauncherSignature::Sign(&key)).unwrap();

        let exe = fixture.dir.path().join("bin").join("voxup");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "voxup 0.1.0").unwrap();
        let outcome = self_update::update_voxup(
            &manifest.to_string_lossy(),
            &made.public_tauri,
            &Version::new(0, 1, 0),
            &exe,
            platform,
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome, VoxupUpdate::Updated { from: Version::new(0, 1, 0), to: Version::new(0, 2, 0) });
        assert_eq!(fs::read_to_string(&exe).unwrap(), "voxup 0.2.0");
    }

    #[test]
    fn checks_a_tauri_signature_names_the_version() {
        let fixture = Fixture::new();
        let (key, _) = fixture.key("launcher", true);
        let bundle = fixture.dir.path().join("Launcher.AppImage");
        fs::write(&bundle, "appimage").unwrap();
        let sig = fixture.dir.path().join("Launcher.AppImage.sig");
        fs::write(&sig, STANDARD.encode(key.sign_file(&bundle, &Version::new(0, 2, 0)).unwrap())).unwrap();

        let mut build = LauncherBuild {
            channel: UpdateChannel::Beta,
            version: Version::new(0, 3, 0),
            target: "linux-x86_64".to_owned(),
            tag: None,
            download_base: None,
            notes: None,
        };
        assert!(add_launcher(&fixture.site, &bundle, &build, LauncherSignature::TauriSig(&sig)).is_err());
        build.version = Version::new(0, 2, 0);
        let path = add_launcher(&fixture.site, &bundle, &build, LauncherSignature::TauriSig(&sig)).unwrap();
        let manifest = UpdateManifest::parse(&fs::read_to_string(path).unwrap()).unwrap();
        assert!(manifest.platforms["linux-x86_64"].url.ends_with("/launcher-v0.2.0/Launcher.AppImage"));
    }

    #[test]
    fn lists_the_manual_as_a_module_of_its_release() {
        let fixture = Fixture::new();
        let (key, made) = fixture.key("engine", false);
        let version = Version::parse("0.4.0-nightly.20261101").unwrap();
        let engine = fixture.archive("0.4.0-nightly.20261101", Channel::Nightly);
        let manual = fixture.dir.path().join("build").join("voxear-0.4.0-nightly.20261101-manual.tar.zst");
        fs::write(&manual, voxear_launcher::testing::manual_archive(&version)).unwrap();
        let file = ModuleFile { version: version.clone(), module: Module::Manual, platform: None };
        assert_eq!(module_of(&manual), Some(file));

        // The manual first, as a glob might give it: the engine archive is still listed before it.
        let added = add_archives(&fixture.site, &[manual.clone(), engine], &key, &fixture.local()).unwrap();
        assert!(matches!(added[0], Added::Engine(_)));
        assert!(matches!(
            &added[1],
            Added::Module { module, .. } if module.name == Module::Manual && module.title == "Voxear Manual"
        ));
        assert!(signature_path(&manual).is_file());

        // The launcher installs the engine with its manual, checking both.
        let mut keys = TrustedKeys::new();
        keys.add(&made.public_line).unwrap();
        let layout = Layout::new(fixture.dir.path().join("home"));
        let url = engine_manifest_path(&fixture.site).to_string_lossy().into_owned();
        let loaded = releases::load(&url, &keys, &layout).unwrap();
        let store = Store::new(layout, Platform::current().unwrap());
        let release = loaded.manifest.find(&version).unwrap();
        let install = store.install(release, &loaded.url, &keys, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert!(install.has_module(Module::Manual));
    }

    #[test]
    fn lists_the_server_on_its_platforms_download() {
        let fixture = Fixture::new();
        let (key, made) = fixture.key("engine", false);
        let version = Version::parse("0.4.0-nightly.20261101").unwrap();
        let platform = Platform::current().unwrap();
        let engine = fixture.archive("0.4.0-nightly.20261101", Channel::Nightly);
        let server = fixture.dir.path().join("build").join(Module::Server.archive_name(&version, platform));
        fs::write(&server, voxear_launcher::testing::server_archive(&version, platform)).unwrap();
        let file = ModuleFile { version: version.clone(), module: Module::Server, platform: Some(platform) };
        assert_eq!(module_of(&server), Some(file));

        let added = add_archives(&fixture.site, &[server.clone(), engine.clone()], &key, &fixture.local()).unwrap();
        assert!(matches!(&added[1], Added::Module { module, .. } if module.name == Module::Server));
        assert!(signature_path(&server).is_file());

        // Adding the engine archive again keeps its server listed.
        add_engine(&fixture.site, &engine, &key, &fixture.local()).unwrap();
        let listed = load_engine_manifest(&fixture.site).unwrap();
        let release = listed.find(&version).unwrap();
        assert!(release.modules.is_empty(), "the server isn't listed for every platform");
        assert_eq!(release.module(Module::Server, platform).unwrap().size, fs::metadata(&server).unwrap().len());

        let mut keys = TrustedKeys::new();
        keys.add(&made.public_line).unwrap();
        let layout = Layout::new(fixture.dir.path().join("home"));
        let url = engine_manifest_path(&fixture.site).to_string_lossy().into_owned();
        let loaded = releases::load(&url, &keys, &layout).unwrap();
        let store = Store::new(layout, platform);
        let release = loaded.manifest.find(&version).unwrap();
        let install = store.install(release, &loaded.url, &keys, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert!(install.has_module(Module::Server));
    }

    #[test]
    fn refuses_a_module_without_its_release_or_outside_its_folder() {
        let fixture = Fixture::new();
        let (key, _) = fixture.key("engine", false);
        let lonely = fixture.dir.path().join("voxear-9.9.9-manual.tar.zst");
        fs::write(&lonely, voxear_launcher::testing::manual_archive(&Version::new(9, 9, 9))).unwrap();
        let error = add_archives(&fixture.site, &[lonely], &key, &fixture.local()).err().unwrap();
        assert!(error.to_string().contains("isn't in the manifest yet"), "{error}");

        add_engine(&fixture.site, &fixture.archive("0.3.0", Channel::Stable), &key, &fixture.local()).unwrap();
        let sneaky = fixture.dir.path().join("voxear-0.3.0-manual.tar.zst");
        let entries = [voxear_launcher::testing::ArchiveEntry::file("bin/vox", b"x", 0o755)];
        fs::write(&sneaky, voxear_launcher::testing::tar_zst(&entries)).unwrap();
        let error = add_archives(&fixture.site, &[sneaky], &key, &fixture.local()).err().unwrap();
        assert!(format!("{error:#}").contains("may only hold manual"), "{error:#}");

        let server = fixture.dir.path().join("voxear-0.3.0-windows-x86_64-server.tar.zst");
        let entries = [voxear_launcher::testing::ArchiveEntry::file("bin/voxear-server.exe", b"x", 0o755)];
        fs::write(&server, voxear_launcher::testing::tar_zst(&entries)).unwrap();
        let error = add_archives(&fixture.site, &[server], &key, &fixture.local()).err().unwrap();
        assert!(error.to_string().contains("for windows-x86_64, which has no engine archive"), "{error}");
        assert!(module_of(Path::new("voxear-0.3.0-server.tar.zst")).is_none(), "the server needs its platform");
    }

    #[test]
    fn never_overwrites_a_key() {
        let fixture = Fixture::new();
        fixture.key("engine", false);
        assert!(keygen(&fixture.dir.path().join("keys").join("engine"), Some("x".to_owned()), false).is_err());
    }

    impl Fixture {
        fn archive_path_for(&self, version: &str) -> PathBuf {
            let platform = Platform::current().unwrap();
            self.dir.path().join("build").join(format!("voxear-{version}-{platform}.tar.zst"))
        }
    }
}
