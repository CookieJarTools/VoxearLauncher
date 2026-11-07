//! Engine versions installed side by side.
//!
//! An install is downloaded, checked against the manifest's size and SHA-256 and against its signature, and
//! only then unpacked, into a staging folder. Once it is complete it is renamed into `engines/<version>` in one
//! step, so an engine folder is either whole or absent, even if the launcher is killed halfway through.
//! Removing works the same way in reverse: the folder is renamed into the trash before it is deleted.

use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use jiff::Timestamp;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive;
use crate::channel::{Channel, Selector};
use crate::engine::{self, EngineInfo};
use crate::fetch;
use crate::fsutil::{self, TempDir};
use crate::layout::Layout;
use crate::manifest::{Module, ModuleDownload, Release};
use crate::platform::Platform;
use crate::signing::TrustedKeys;

/// Written into each install the launcher makes. Its presence is what marks a folder in `engines/` as a
/// finished install.
pub const RECEIPT_FILE: &str = ".voxup-receipt.toml";
const LOCK_WAIT: Duration = Duration::from_millis(750);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed by the launcher, in its own `engines/` folder.
    Managed,
    /// Somewhere else, such as an engine built from source; the launcher was pointed at it.
    Located,
}

#[derive(Clone, Debug)]
pub struct Install {
    pub version: Version,
    pub channel: Channel,
    /// The install folder: this version's `VOXEAR_HOME`.
    pub path: PathBuf,
    pub kind: InstallKind,
    /// When the launcher installed it. Unknown for located installs.
    pub installed: Option<Timestamp>,
    /// A located install whose folder has gone, for example from a drive that isn't mounted.
    pub missing: bool,
}

impl Install {
    pub fn vox(&self) -> PathBuf {
        engine::vox(&self.path)
    }

    pub fn editor(&self) -> PathBuf {
        engine::editor(&self.path)
    }

    /// Whether an optional part, such as the manual, is in this install.
    pub fn has_module(&self, module: Module) -> bool {
        Platform::current().is_some_and(|platform| self.path.join(module.path(platform)).exists())
    }
}

/// How an install is getting on, for progress bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Downloading {
        done: u64,
        total: u64,
    },
    Verifying,
    /// `done` and `total` count compressed bytes.
    Unpacking {
        done: u64,
        total: u64,
    },
    Finishing,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Removed {
    /// The launcher's own install, now deleted.
    Deleted,
    /// A located install: the launcher forgot it and left its files alone.
    Forgotten(PathBuf),
}

/// Returned when an install is cancelled, so a UI can tell it apart from a failure.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug, Serialize, Deserialize)]
struct Receipt {
    version: Version,
    channel: Channel,
    platform: String,
    /// The manifest URL or archive path it came from.
    source: String,
    sha256: String,
    installed: Timestamp,
}

#[derive(Default, Serialize, Deserialize)]
struct LocatedFile {
    #[serde(default, rename = "install")]
    installs: Vec<LocatedEntry>,
}

/// The version is saved so a located install still shows what it was while its folder is unavailable.
#[derive(Clone, Serialize, Deserialize)]
struct LocatedEntry {
    path: PathBuf,
    version: Version,
    channel: Channel,
}

pub struct Store {
    layout: Layout,
    platform: Platform,
}

impl Store {
    pub fn new(layout: Layout, platform: Platform) -> Self {
        Self { layout, platform }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Every engine version available, newest first.
    pub fn list(&self) -> Result<Vec<Install>> {
        let mut installs = self.managed()?;
        installs.extend(self.located()?);
        installs.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(installs)
    }

    pub fn get(&self, version: &Version) -> Result<Option<Install>> {
        Ok(self.list()?.into_iter().find(|install| &install.version == version))
    }

    /// The newest install `selector` matches, skipping located installs whose folder has gone.
    pub fn pick(&self, selector: &Selector) -> Result<Option<Install>> {
        let mut installs = self.list()?.into_iter().filter(|install| !install.missing);
        Ok(installs.find(|install| match selector {
            Selector::Latest(channel) => install.channel == *channel,
            Selector::Exact(version) => &install.version == version,
            Selector::Matching(req) => req.matches(&install.version),
        }))
    }

    /// The newest stable version installed, for when nothing else says which version to use.
    pub fn newest_stable(&self) -> Result<Option<Install>> {
        self.pick(&Selector::Latest(Channel::Stable))
    }

    /// Downloads, verifies and installs a release from a manifest, with all of its optional modules. `manifest_url`
    /// is where the manifest came from, which download URLs may be relative to.
    pub fn install(
        &self,
        release: &Release,
        manifest_url: &str,
        keys: &TrustedKeys,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<Install> {
        self.install_without(release, manifest_url, keys, &[], progress, cancel)
    }

    /// Installs a release with its optional modules except those in `without`. The engine and each module
    /// are downloaded and checked against the manifest and their signatures before any of them is unpacked, and
    /// the install appears in one step, modules and all.
    pub fn install_without(
        &self,
        release: &Release,
        manifest_url: &str,
        keys: &TrustedKeys,
        without: &[Module],
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<Install> {
        let _lock = self.lock()?;
        self.clear_leftovers();
        let version = &release.version;
        self.ensure_not_installed(version)?;
        if release.needs_newer_launcher() {
            let min = release.min_launcher.as_ref().expect("needs_newer_launcher checks it");
            bail!("Voxear {version} needs launcher {min} or newer; update the launcher first");
        }
        let download = release
            .download(self.platform)
            .ok_or_else(|| anyhow!("Voxear {version} has no download for {}", self.platform))?;
        let modules: Vec<&ModuleDownload> =
            release.offered(self.platform).into_iter().filter(|module| !without.contains(&module.name)).collect();
        let total = download.size + modules.iter().map(|module| module.size).sum::<u64>();

        let engine = Part {
            what: format!("Voxear {version}"),
            url: fetch::resolve(manifest_url, &download.url),
            file: self.layout.downloads().join(format!("voxear-{version}-{}.tar.zst.part", self.platform)),
            size: download.size,
            sha256: &download.sha256,
            signature: &download.signature,
        };
        let module_parts: Vec<(Module, Part)> =
            modules.iter().map(|module| (module.name, self.module_part(version, module, manifest_url))).collect();

        let result = (|| {
            let mut done = 0;
            let sha256 = engine.fetch(keys, done, total, progress, cancel)?;
            done += engine.size;
            for (_, part) in &module_parts {
                part.fetch(keys, done, total, progress, cancel)?;
                done += part.size;
            }
            let modules: Vec<(Module, &Path)> =
                module_parts.iter().map(|(module, part)| (*module, part.file.as_path())).collect();
            self.unpack_and_commit(
                &engine.file,
                Some((version, release.channel)),
                engine.url.clone(),
                sha256,
                &modules,
                progress,
            )
        })();
        _ = fs::remove_file(&engine.file);
        for (_, part) in &module_parts {
            _ = fs::remove_file(&part.file);
        }
        result
    }

    /// Adds one of a release's optional modules to its install, checked the same way as the engine. Only the
    /// launcher's own installs can change: a located engine's files are left alone.
    pub fn add_module(
        &self,
        release: &Release,
        module: Module,
        manifest_url: &str,
        keys: &TrustedKeys,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<Install> {
        let _lock = self.lock()?;
        self.clear_leftovers();
        let version = &release.version;
        let install = self.managed_install_of(version)?;
        let listed = release
            .module(module, self.platform)
            .ok_or_else(|| anyhow!("Voxear {version} has no {module} for {}", self.platform))?;
        let path = module.path(self.platform);
        let target = install.path.join(&path);
        ensure!(!target.exists(), "Voxear {version} already has the {module}");

        let part = self.module_part(version, listed, manifest_url);
        let result = (|| {
            part.fetch(keys, 0, part.size, progress, cancel)?;
            let staging = TempDir::create(self.layout.staging().join(fsutil::unique_suffix()))?;
            progress(Progress::Unpacking { done: 0, total: part.size });
            archive::unpack_module(&part.file, staging.path(), module, self.platform, &mut |done| {
                progress(Progress::Unpacking { done, total: part.size })
            })?;
            progress(Progress::Finishing);
            fs::rename(staging.path().join(&path), &target)
                .with_context(|| format!("couldn't move the {module} into {}", target.display()))
        })();
        _ = fs::remove_file(&part.file);
        result?;
        Ok(install)
    }

    /// Removes an optional part from one of the launcher's installs: the manual's folder, or the server's one file.
    pub fn remove_module(&self, version: &Version, module: Module) -> Result<()> {
        let _lock = self.lock()?;
        let install = self.managed_install_of(version)?;
        let path = install.path.join(module.path(self.platform));
        ensure!(path.exists(), "Voxear {version} doesn't have the {module}");
        self.move_to_trash(&path)
            .with_context(|| format!("couldn't remove the {module}; close anything running from it and try again"))?;
        self.empty_trash();
        Ok(())
    }

    fn module_part<'a>(&self, version: &Version, module: &'a ModuleDownload, manifest_url: &str) -> Part<'a> {
        let file = module.name.archive_name(version, self.platform);
        Part {
            what: format!("the {} for Voxear {version}", module.title),
            url: fetch::resolve(manifest_url, &module.url),
            file: self.layout.downloads().join(format!("{file}.part")),
            size: module.size,
            sha256: &module.sha256,
            signature: &module.signature,
        }
    }

    fn managed_install_of(&self, version: &Version) -> Result<Install> {
        let install = self.get(version)?.ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
        ensure!(
            install.kind == InstallKind::Managed,
            "Voxear {version} was located, not installed by the launcher, so its files are left as they are"
        );
        Ok(install)
    }

    /// Installs an engine archive from disk, for offline machines. `signature` is the contents of its
    /// `.minisig` file. The version comes from the `engine.toml` inside the archive.
    pub fn install_archive(
        &self,
        archive: &Path,
        signature: &str,
        keys: &TrustedKeys,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Install> {
        let _lock = self.lock()?;
        self.clear_leftovers();
        progress(Progress::Verifying);
        keys.verify_file(archive, signature)
            .with_context(|| format!("{} failed its signature check", archive.display()))?;
        let source = fsutil::absolute(archive)?.to_string_lossy().into_owned();
        self.unpack_and_commit(archive, None, source, sha256_file(archive)?, &[], progress)
    }

    /// Records an engine that lives somewhere else, such as one built from source. Its files are never
    /// changed or deleted by the launcher.
    pub fn locate(&self, dir: &Path) -> Result<Install> {
        let _lock = self.lock()?;
        let dir = fsutil::absolute(dir)?;
        let info = EngineInfo::read(&dir)?;
        ensure!(engine::vox(&dir).is_file(), "{} has no {}", dir.display(), engine::vox(Path::new("")).display());
        ensure!(
            info.platform == self.platform.as_str(),
            "{} is a {} engine, and this is {}",
            dir.display(),
            info.platform,
            self.platform
        );
        if let Some(known) = self.list()?.into_iter().find(|install| fsutil::same_path(&install.path, &dir)) {
            bail!("Voxear {} at {} is already in the list", known.version, dir.display());
        }
        self.ensure_not_installed(&info.version)?;

        let mut file = self.load_located()?;
        file.installs.push(LocatedEntry { path: dir.clone(), version: info.version.clone(), channel: info.channel });
        self.save_located(&file)?;
        Ok(Install {
            version: info.version,
            channel: info.channel,
            path: dir,
            kind: InstallKind::Located,
            installed: None,
            missing: false,
        })
    }

    /// Deletes one of the launcher's installs, or forgets a located one without touching its files.
    pub fn remove(&self, version: &Version) -> Result<Removed> {
        let _lock = self.lock()?;
        let install = self.get(version)?.ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
        if install.kind == InstallKind::Located {
            let mut file = self.load_located()?;
            file.installs.retain(|entry| !fsutil::same_path(&entry.path, &install.path));
            self.save_located(&file)?;
            return Ok(Removed::Forgotten(install.path));
        }
        self.move_to_trash(&install.path).with_context(|| {
            format!("couldn't remove Voxear {version}; close the editor and any game running from it, then try again")
        })?;
        self.empty_trash();
        Ok(Removed::Deleted)
    }

    /// Unpacks a verified archive and moves it into place. `expected` is the version and channel the manifest
    /// promised; an archive installed from disk is whatever its `engine.toml` says.
    fn unpack_and_commit(
        &self,
        archive_path: &Path,
        expected: Option<(&Version, Channel)>,
        source: String,
        sha256: String,
        modules: &[(Module, &Path)],
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Install> {
        let size = |path: &Path| fs::metadata(path).map(|meta| meta.len()).unwrap_or_default();
        let engine_size = size(archive_path);
        let total = engine_size + modules.iter().map(|(_, path)| size(path)).sum::<u64>();
        let staging = TempDir::create(self.layout.staging().join(fsutil::unique_suffix()))?;
        progress(Progress::Unpacking { done: 0, total });
        archive::unpack(archive_path, staging.path(), &mut |done| progress(Progress::Unpacking { done, total }))?;

        let info = EngineInfo::read(staging.path()).context("the archive isn't a Voxear engine")?;
        ensure!(
            info.platform == self.platform.as_str(),
            "the archive holds a {} engine, and this is {}",
            info.platform,
            self.platform
        );
        match expected {
            Some((version, channel)) => ensure!(
                info.version == *version && info.channel == channel,
                "the archive for Voxear {version} {channel} holds Voxear {} {} instead",
                info.version,
                info.channel
            ),
            None => self.ensure_not_installed(&info.version)?,
        }
        ensure!(engine::vox(staging.path()).is_file(), "the archive for Voxear {} has no `vox`", info.version);

        let mut done = engine_size;
        for &(module, path) in modules {
            // An engine archive from before modules existed may carry the part itself.
            if !staging.path().join(module.path(self.platform)).exists() {
                archive::unpack_module(path, staging.path(), module, self.platform, &mut |read| {
                    progress(Progress::Unpacking { done: done + read, total })
                })?;
            }
            done += size(path);
        }

        progress(Progress::Finishing);
        let receipt = Receipt {
            version: info.version,
            channel: info.channel,
            platform: info.platform,
            source,
            sha256,
            installed: Timestamp::now(),
        };
        fsutil::write_atomic(&staging.path().join(RECEIPT_FILE), toml::to_string(&receipt)?.as_bytes())?;
        let target = self.layout.engine(&receipt.version);
        if target.exists() {
            // A folder without a receipt: left by hand, or by something other than the launcher.
            self.move_to_trash(&target)?;
        }
        fs::create_dir_all(self.layout.engines())?;
        fs::rename(staging.path(), &target)
            .with_context(|| format!("couldn't move the engine into {}", target.display()))?;
        staging.keep();
        self.empty_trash();
        Ok(managed_install(receipt, target))
    }

    fn ensure_not_installed(&self, version: &Version) -> Result<()> {
        match self.get(version)? {
            Some(install) => bail!("Voxear {version} is already installed, at {}", install.path.display()),
            None => Ok(()),
        }
    }

    fn managed(&self) -> Result<Vec<Install>> {
        let dir = self.layout.engines();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("couldn't read {}", dir.display())),
        };
        let mut installs = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if let Some(receipt) = read_receipt(&path) {
                installs.push(managed_install(receipt, path));
            }
        }
        Ok(installs)
    }

    /// Located installs, with what their `engine.toml` says now when the folder is there.
    fn located(&self) -> Result<Vec<Install>> {
        let installs = self.load_located()?.installs.into_iter().map(|entry| {
            let live = EngineInfo::read(&entry.path).ok();
            Install {
                version: live.as_ref().map_or(entry.version, |info| info.version.clone()),
                channel: live.as_ref().map_or(entry.channel, |info| info.channel),
                missing: live.is_none() || !engine::vox(&entry.path).is_file(),
                path: entry.path,
                kind: InstallKind::Located,
                installed: None,
            }
        });
        Ok(installs.collect())
    }

    fn load_located(&self) -> Result<LocatedFile> {
        let path = self.layout.located_file();
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("{} isn't valid", path.display())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(LocatedFile::default()),
            Err(e) => Err(e).with_context(|| format!("couldn't read {}", path.display())),
        }
    }

    fn save_located(&self, file: &LocatedFile) -> Result<()> {
        fsutil::write_atomic(&self.layout.located_file(), toml::to_string(file)?.as_bytes())
    }

    fn move_to_trash(&self, path: &Path) -> Result<()> {
        let trash = self.layout.trash();
        fs::create_dir_all(&trash).with_context(|| format!("couldn't create {}", trash.display()))?;
        fs::rename(path, trash.join(fsutil::unique_suffix()))
            .with_context(|| format!("couldn't move {} out of the way", path.display()))
    }

    /// Best effort: anything still in use is retried the next time.
    fn empty_trash(&self) {
        clear_folder(&self.layout.trash());
    }

    /// Removes what a launcher that was killed mid-install left behind. Only called with the lock held, so
    /// nothing else is using these folders.
    fn clear_leftovers(&self) {
        clear_folder(&self.layout.downloads());
        clear_folder(&self.layout.staging());
        self.empty_trash();
    }

    /// Only one launcher (window or command line) changes the installs at a time.
    fn lock(&self) -> Result<File> {
        let root = self.layout.root();
        fs::create_dir_all(root).with_context(|| format!("couldn't create {}", root.display()))?;
        let path = self.layout.lock_file();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("couldn't open {}", path.display()))?;
        // Waits a moment first: a quick change by another launcher finishes in that time, and so does a child
        // process that was started while the lock was held and still shares it until it has fully started.
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(TryLockError::WouldBlock) => {
                    bail!("another launcher is installing or removing an engine; try again once it has finished")
                }
                Err(TryLockError::Error(e)) => {
                    return Err(e).with_context(|| format!("couldn't lock {}", path.display()));
                }
            }
        }
    }
}

fn clear_folder(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
    }
}

fn read_receipt(dir: &Path) -> Option<Receipt> {
    let text = fs::read_to_string(dir.join(RECEIPT_FILE)).ok()?;
    let receipt: Receipt = toml::from_str(&text).ok()?;
    // A folder renamed by hand would otherwise shadow the version it claims.
    (dir.file_name()? == receipt.version.to_string().as_str()).then_some(receipt)
}

fn managed_install(receipt: Receipt, path: PathBuf) -> Install {
    Install {
        version: receipt.version,
        channel: receipt.channel,
        path,
        kind: InstallKind::Managed,
        installed: Some(receipt.installed),
        missing: false,
    }
}

/// One file of an install, as the manifest describes it.
struct Part<'a> {
    /// What it is, for messages: "Voxear 0.3.0", or "the Voxear Manual for Voxear 0.3.0".
    what: String,
    url: String,
    /// Where it downloads to.
    file: PathBuf,
    size: u64,
    sha256: &'a str,
    signature: &'a str,
}

impl Part<'_> {
    /// Downloads the file and checks its size, SHA-256 and signature. Progress counts from `offset` towards
    /// `total`, so an engine and its modules fill one bar. Returns the SHA-256.
    fn fetch(
        &self,
        keys: &TrustedKeys,
        offset: u64,
        total: u64,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<String> {
        let mut report = |step| match step {
            Progress::Downloading { done, .. } => progress(Progress::Downloading { done: offset + done, total }),
            other => progress(other),
        };
        let sha256 = download_to(&self.url, &self.file, self.size, &mut report, cancel)?;
        ensure!(
            sha256.eq_ignore_ascii_case(self.sha256),
            "the download of {} is corrupt (its SHA-256 doesn't match the manifest), so it was thrown away",
            self.what
        );
        progress(Progress::Verifying);
        keys.verify_file(&self.file, self.signature)
            .with_context(|| format!("the download of {} failed its signature check", self.what))?;
        Ok(sha256)
    }
}

/// Streams `url` to `part`, refusing more bytes than the manifest promised, and returns its SHA-256.
fn download_to(
    url: &str,
    part: &Path,
    size: u64,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<String> {
    if let Some(parent) = part.parent() {
        fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    let mut reader = fetch::open(url)?.reader;
    let mut file = File::create(part).with_context(|| format!("couldn't write {}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut last_report = Instant::now();
    progress(Progress::Downloading { done, total: size });
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let read = reader.read(&mut buffer).context("the download stopped")?;
        if read == 0 {
            break;
        }
        done += read as u64;
        ensure!(done <= size, "the download is larger than the manifest says it should be, so it was stopped");
        file.write_all(&buffer[..read]).with_context(|| format!("couldn't write {}", part.display()))?;
        hasher.update(&buffer[..read]);
        if last_report.elapsed() >= Duration::from_millis(80) {
            progress(Progress::Downloading { done, total: size });
            last_report = Instant::now();
        }
    }
    progress(Progress::Downloading { done, total: size });
    file.sync_all().context("couldn't finish writing the download")?;
    ensure!(done == size, "the download was cut short ({done} of {size} bytes)");
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).with_context(|| format!("couldn't read {}", path.display()))?;
        if read == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ArchiveEntry, FakeReleases, TestKey, engine_archive, tar_zst};

    struct Fixture {
        _dir: tempfile::TempDir,
        fake: FakeReleases,
        store: Store,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let fake = FakeReleases::new(dir.path().join("mirror"));
            let store = Store::new(Layout::new(dir.path().join("home")), platform());
            Self { _dir: dir, fake, store }
        }

        fn install(&self, version: &str) -> Result<Install> {
            self.install_reporting(version, &mut |_| {})
        }

        fn install_reporting(&self, version: &str, progress: &mut dyn FnMut(Progress)) -> Result<Install> {
            let url = self.fake.write();
            let release = self.fake.manifest.find(&v(version)).unwrap();
            self.store.install(release, &url, &self.fake.trusted_keys(), progress, &AtomicBool::new(false))
        }

        fn install_without(&self, version: &str, without: &[Module]) -> Result<Install> {
            let url = self.fake.write();
            let release = self.fake.manifest.find(&v(version)).unwrap();
            let keys = self.fake.trusted_keys();
            self.store.install_without(release, &url, &keys, without, &mut |_| {}, &AtomicBool::new(false))
        }

        fn add_later(&self, version: &str, module: Module) -> Result<Install> {
            let url = self.fake.write();
            let release = self.fake.manifest.find(&v(version)).unwrap();
            let keys = self.fake.trusted_keys();
            self.store.add_module(release, module, &url, &keys, &mut |_| {}, &AtomicBool::new(false))
        }

        fn download(&mut self, version: &str) -> &mut crate::manifest::Download {
            let platform = platform().as_str();
            let release = self.fake.release(&v(version));
            release.downloads.iter_mut().find(|download| download.platform == platform).unwrap()
        }

        fn assert_nothing_left_behind(&self) {
            let layout = self.store.layout();
            assert!(self.store.list().unwrap().is_empty(), "nothing is installed");
            for dir in [layout.downloads(), layout.staging(), layout.trash()] {
                let leftovers = fs::read_dir(&dir).map(|entries| entries.count()).unwrap_or(0);
                assert_eq!(leftovers, 0, "{} is empty", dir.display());
            }
        }
    }

    fn platform() -> Platform {
        Platform::current().expect("tests run on a platform Voxear supports")
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn error_text(result: Result<Install>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    #[test]
    fn installs_a_release_and_lists_it() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let mut steps = Vec::new();
        let install = fixture.install_reporting("0.3.0", &mut |step| steps.push(step)).unwrap();

        assert_eq!(install.version, v("0.3.0"));
        assert_eq!(install.kind, InstallKind::Managed);
        assert_eq!(install.path, fixture.store.layout().engine(&v("0.3.0")));
        assert!(install.vox().is_file());
        assert!(install.path.join(RECEIPT_FILE).is_file());
        assert_eq!(EngineInfo::read(&install.path).unwrap().version, v("0.3.0"));

        let listed = fixture.store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].version, v("0.3.0"));
        assert!(listed[0].installed.is_some());

        assert!(matches!(steps.first(), Some(Progress::Downloading { done: 0, .. })));
        assert!(steps.contains(&Progress::Verifying));
        assert!(
            steps.iter().any(|step| matches!(step, Progress::Unpacking { done, total } if done > &0 && done <= total))
        );
        assert_eq!(steps.last(), Some(&Progress::Finishing));
        assert_eq!(fs::read_dir(fixture.store.layout().downloads()).unwrap().count(), 0, "the archive is deleted");
    }

    #[test]
    fn installs_versions_side_by_side() {
        let mut fixture = Fixture::new();
        for (version, channel) in
            [("0.2.0", Channel::Stable), ("0.3.0-beta.1", Channel::Beta), ("0.3.0", Channel::Stable)]
        {
            fixture.fake.add(version, channel);
            fixture.install(version).unwrap();
        }
        let versions: Vec<_> = fixture.store.list().unwrap().iter().map(|i| i.version.to_string()).collect();
        assert_eq!(versions, ["0.3.0", "0.3.0-beta.1", "0.2.0"]);
        assert_eq!(fixture.store.newest_stable().unwrap().unwrap().version, v("0.3.0"));
        assert_eq!(fixture.store.get(&v("0.2.0")).unwrap().unwrap().channel, Channel::Stable);
    }

    #[test]
    fn picks_installed_versions_like_releases() {
        let mut fixture = Fixture::new();
        for (version, channel) in
            [("0.2.0", Channel::Stable), ("0.2.1", Channel::Stable), ("0.3.0-beta.1", Channel::Beta)]
        {
            fixture.fake.add(version, channel);
            fixture.install(version).unwrap();
        }
        let pick = |s: &str| fixture.store.pick(&s.parse().unwrap()).unwrap().map(|i| i.version.to_string());
        assert_eq!(pick("stable").as_deref(), Some("0.2.1"));
        assert_eq!(pick("beta").as_deref(), Some("0.3.0-beta.1"));
        assert_eq!(pick("0.2.0").as_deref(), Some("0.2.0"));
        assert_eq!(pick("0.2").as_deref(), Some("0.2.1"));
        assert_eq!(pick("nightly"), None);
    }

    #[test]
    fn clears_what_a_killed_install_left_behind() {
        let mut fixture = Fixture::new();
        let layout = fixture.store.layout().clone();
        fs::create_dir_all(layout.staging().join("12345-dead")).unwrap();
        fs::create_dir_all(layout.downloads()).unwrap();
        fs::write(layout.downloads().join("voxear-0.1.0.tar.zst.part"), "half").unwrap();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.install("0.3.0").unwrap();
        assert_eq!(fs::read_dir(layout.staging()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(layout.downloads()).unwrap().count(), 0);
    }

    #[test]
    fn installs_the_manual_with_the_engine() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        let mut steps = Vec::new();
        let install = fixture.install_reporting("0.3.0", &mut |step| steps.push(step)).unwrap();
        assert!(install.has_module(Module::Manual));
        assert!(install.path.join("manual/index.md").is_file());
        assert!(install.vox().is_file());

        // One bar for the engine and its manual together.
        let release = fixture.fake.manifest.find(&v("0.3.0")).unwrap();
        let manual = release.module(Module::Manual, platform()).unwrap();
        let total = release.download(platform()).unwrap().size + manual.size;
        let downloads: Vec<_> = steps
            .iter()
            .filter_map(|step| match step {
                Progress::Downloading { done, total } => Some((*done, *total)),
                _ => None,
            })
            .collect();
        assert!(downloads.iter().all(|(_, t)| *t == total));
        assert_eq!(downloads.last().unwrap().0, total);
        assert_eq!(fs::read_dir(fixture.store.layout().downloads()).unwrap().count(), 0);
    }

    #[test]
    fn installs_without_the_manual_when_asked() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        let install = fixture.install_without("0.3.0", &[Module::Manual]).unwrap();
        assert!(!install.has_module(Module::Manual));
        assert!(install.vox().is_file());
    }

    #[test]
    fn adds_and_removes_the_manual_later() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        let install = fixture.install_without("0.3.0", &[Module::Manual]).unwrap();

        fixture.add_later("0.3.0", Module::Manual).unwrap();
        assert!(install.has_module(Module::Manual));
        assert!(error_text(fixture.add_later("0.3.0", Module::Manual)).contains("already has the Voxear Manual"));

        fixture.store.remove_module(&v("0.3.0"), Module::Manual).unwrap();
        assert!(!install.has_module(Module::Manual));
        assert!(install.vox().is_file(), "the engine stays");
        assert!(fixture.store.remove_module(&v("0.3.0"), Module::Manual).is_err());
    }

    #[test]
    fn installs_the_server_into_bin_with_the_engine() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        fixture.fake.add_server("0.3.0");
        let install = fixture.install("0.3.0").unwrap();
        let server = install.path.join(Module::Server.path(platform()));
        assert!(server.is_file());
        assert!(install.has_module(Module::Server) && install.has_module(Module::Manual));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&server).unwrap().permissions().mode() & 0o111, 0o111, "it stays executable");
        }
    }

    #[test]
    fn adds_and_removes_the_server_later() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        fixture.fake.add_server("0.3.0");
        let install = fixture.install_without("0.3.0", &[Module::Server]).unwrap();
        assert!(!install.has_module(Module::Server));
        assert!(install.has_module(Module::Manual));

        fixture.add_later("0.3.0", Module::Server).unwrap();
        assert!(install.has_module(Module::Server));
        assert!(error_text(fixture.add_later("0.3.0", Module::Server)).contains("already has the Dedicated Server"));

        let bin_before = fs::read_dir(install.path.join("bin")).unwrap().count();
        fixture.store.remove_module(&v("0.3.0"), Module::Server).unwrap();
        assert!(!install.has_module(Module::Server));
        assert_eq!(fs::read_dir(install.path.join("bin")).unwrap().count(), bin_before - 1, "only its own file goes");
        assert!(install.vox().is_file());
        assert!(install.has_module(Module::Manual));
    }

    #[test]
    fn says_when_a_release_has_no_server() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.install("0.3.0").unwrap();
        assert!(error_text(fixture.add_later("0.3.0", Module::Server)).contains("has no Dedicated Server"));
    }

    #[test]
    fn refuses_a_server_that_would_change_engine_files() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let sneaky = tar_zst(&[ArchiveEntry::dir("bin"), ArchiveEntry::file("bin/vox", b"x", 0o755)]);
        fixture.fake.add_module(&v("0.3.0"), Module::Server, platform(), &sneaky);
        assert!(error_text(fixture.install("0.3.0")).contains("may only hold bin/voxear-server"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn leaves_a_located_engines_files_alone() {
        let mut fixture = Fixture::new();
        let dir = fixture.fake.dir.join("built-from-source");
        let archive = engine_archive(&v("0.3.0"), Channel::Stable, platform());
        archive::unpack_reader(archive.as_slice(), &dir, &mut |_| {}).unwrap();
        fixture.store.locate(&dir).unwrap();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        assert!(error_text(fixture.add_later("0.3.0", Module::Manual)).contains("left as they are"));
        assert!(!dir.join("manual").exists());
    }

    #[test]
    fn refuses_a_manual_that_fails_its_checks() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.fake.add_manual("0.3.0");
        let manual = fs::read(fixture.fake.dir.join("voxear-0.3.0-manual.tar.zst")).unwrap();
        fixture.fake.release(&v("0.3.0")).modules[0].signature = TestKey::generate().sign(&manual);
        assert!(
            error_text(fixture.install("0.3.0")).contains("Voxear Manual for Voxear 0.3.0 failed its signature check")
        );
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn refuses_a_manual_that_would_change_engine_files() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let sneaky =
            tar_zst(&[ArchiveEntry::file("manual/index.md", b"x", 0o644), ArchiveEntry::file("bin/vox", b"x", 0o755)]);
        fixture.fake.add_module(&v("0.3.0"), Module::Manual, platform(), &sneaky);
        assert!(error_text(fixture.install("0.3.0")).contains("may only hold manual"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn refuses_to_install_a_version_twice() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.install("0.3.0").unwrap();
        assert!(error_text(fixture.install("0.3.0")).contains("already installed"));
    }

    #[test]
    fn throws_away_a_download_whose_hash_does_not_match() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.download("0.3.0").sha256 = "0".repeat(64);
        assert!(error_text(fixture.install("0.3.0")).contains("SHA-256"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn rejects_an_archive_signed_by_someone_else() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let archive = fs::read(fixture.fake.archive_path("0.3.0", platform())).unwrap();
        fixture.download("0.3.0").signature = TestKey::generate().sign(&archive);
        assert!(error_text(fixture.install("0.3.0")).contains("signature check"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn rejects_an_archive_changed_after_signing() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let path = fixture.fake.archive_path("0.3.0", platform());
        let mut archive = fs::read(&path).unwrap();
        *archive.last_mut().unwrap() ^= 0xff;
        fs::write(&path, &archive).unwrap();
        // Even a manifest that agrees with the changed archive can't make it pass.
        fixture.download("0.3.0").sha256 = hex(&Sha256::digest(&archive));
        assert!(error_text(fixture.install("0.3.0")).contains("signature check"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn rejects_downloads_of_the_wrong_size() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let size = fixture.download("0.3.0").size;
        fixture.download("0.3.0").size = size - 1;
        assert!(error_text(fixture.install("0.3.0")).contains("larger than the manifest"));
        fixture.download("0.3.0").size = size + 1;
        assert!(error_text(fixture.install("0.3.0")).contains("cut short"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn rejects_an_archive_holding_a_different_version() {
        let mut fixture = Fixture::new();
        let archive = engine_archive(&v("0.2.0"), Channel::Stable, platform());
        fixture.fake.add_archive(&v("0.3.0"), Channel::Stable, platform(), &archive);
        assert!(error_text(fixture.install("0.3.0")).contains("holds Voxear 0.2.0"));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn rejects_an_archive_for_another_platform() {
        let mut fixture = Fixture::new();
        let other = Platform::ALL.into_iter().find(|p| *p != platform()).unwrap();
        let archive = engine_archive(&v("0.3.0"), Channel::Stable, other);
        fixture.fake.add_archive(&v("0.3.0"), Channel::Stable, platform(), &archive);
        assert!(error_text(fixture.install("0.3.0")).contains(&format!("holds a {other} engine")));
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn explains_releases_it_cannot_install() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable).min_launcher = Some(v("99.0.0"));
        assert!(error_text(fixture.install("0.3.0")).contains("update the launcher"));

        fixture.fake.add("0.4.0", Channel::Stable).downloads.retain(|d| d.platform != platform().as_str());
        assert!(error_text(fixture.install("0.4.0")).contains("no download for"));
    }

    #[test]
    fn stops_when_cancelled() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let url = fixture.fake.write();
        let release = fixture.fake.manifest.find(&v("0.3.0")).unwrap();
        let keys = fixture.fake.trusted_keys();
        let error = fixture.store.install(release, &url, &keys, &mut |_| {}, &AtomicBool::new(true)).unwrap_err();
        assert!(error.is::<Cancelled>());
        fixture.assert_nothing_left_behind();
    }

    #[test]
    fn removes_an_install() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let install = fixture.install("0.3.0").unwrap();
        assert_eq!(fixture.store.remove(&v("0.3.0")).unwrap(), Removed::Deleted);
        assert!(!install.path.exists());
        fixture.assert_nothing_left_behind();
        assert!(fixture.store.remove(&v("0.3.0")).is_err());
    }

    #[test]
    fn replaces_a_folder_that_has_no_receipt() {
        let mut fixture = Fixture::new();
        let stray = fixture.store.layout().engine(&v("0.3.0"));
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join("junk"), "left by hand").unwrap();
        assert!(fixture.store.list().unwrap().is_empty(), "a folder without a receipt isn't an install");

        fixture.fake.add("0.3.0", Channel::Stable);
        let install = fixture.install("0.3.0").unwrap();
        assert!(!install.path.join("junk").exists());
    }

    #[test]
    fn installs_an_archive_from_disk() {
        let fixture = Fixture::new();
        let archive = engine_archive(&v("0.5.0-beta.1"), Channel::Beta, platform());
        let path = fixture.fake.dir.join("offline.tar.zst");
        fs::write(&path, &archive).unwrap();
        let signature = fixture.fake.key.sign(&archive);

        let keys = fixture.fake.trusted_keys();
        let install = fixture.store.install_archive(&path, &signature, &keys, &mut |_| {}).unwrap();
        assert_eq!(install.version, v("0.5.0-beta.1"));
        assert_eq!(install.channel, Channel::Beta);
        assert!(fixture.store.install_archive(&path, &signature, &keys, &mut |_| {}).is_err(), "only once");

        let unsigned = TestKey::generate().sign(&archive);
        fixture.store.remove(&install.version).unwrap();
        assert!(fixture.store.install_archive(&path, &unsigned, &keys, &mut |_| {}).is_err());
    }

    #[test]
    fn locates_an_engine_elsewhere_and_forgets_it_without_deleting_it() {
        let fixture = Fixture::new();
        let dir = fixture.fake.dir.join("built-from-source");
        let archive = engine_archive(&v("0.4.0-nightly.1"), Channel::Nightly, platform());
        archive::unpack_reader(archive.as_slice(), &dir, &mut |_| {}).unwrap();

        let install = fixture.store.locate(&dir).unwrap();
        assert_eq!(install.kind, InstallKind::Located);
        assert_eq!(install.version, v("0.4.0-nightly.1"));
        assert!(fixture.store.locate(&dir).is_err(), "only once");
        assert_eq!(fixture.store.list().unwrap().len(), 1);

        assert_eq!(fixture.store.remove(&install.version).unwrap(), Removed::Forgotten(dir.clone()));
        assert!(fixture.store.list().unwrap().is_empty());
        assert!(dir.join(crate::engine::ENGINE_FILE).is_file(), "its files are left alone");
    }

    #[test]
    fn remembers_a_located_engine_whose_folder_has_gone() {
        let fixture = Fixture::new();
        let dir = fixture.fake.dir.join("usb-drive");
        let archive = engine_archive(&v("0.4.0"), Channel::Stable, platform());
        archive::unpack_reader(archive.as_slice(), &dir, &mut |_| {}).unwrap();
        fixture.store.locate(&dir).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        let listed = fixture.store.list().unwrap();
        assert_eq!(listed[0].version, v("0.4.0"));
        assert!(listed[0].missing);
        assert!(fixture.store.newest_stable().unwrap().is_none(), "a missing engine is never picked");
    }

    #[test]
    fn refuses_to_locate_something_that_is_not_an_engine() {
        let fixture = Fixture::new();
        let error = fixture.store.locate(&fixture.fake.dir).unwrap_err();
        assert!(error.to_string().contains("isn't a Voxear engine"), "{error}");
    }

    #[test]
    fn refuses_a_located_engine_with_a_version_already_installed() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        fixture.install("0.3.0").unwrap();
        let dir = fixture.fake.dir.join("copy");
        let archive = engine_archive(&v("0.3.0"), Channel::Stable, platform());
        archive::unpack_reader(archive.as_slice(), &dir, &mut |_| {}).unwrap();
        assert!(fixture.store.locate(&dir).unwrap_err().to_string().contains("already installed"));
    }

    #[test]
    fn only_one_launcher_changes_installs_at_a_time() {
        let mut fixture = Fixture::new();
        fixture.fake.add("0.3.0", Channel::Stable);
        let held = fixture.store.lock().unwrap();
        assert!(error_text(fixture.install("0.3.0")).contains("another launcher"));
        drop(held);
        fixture.install("0.3.0").unwrap();
    }
}
