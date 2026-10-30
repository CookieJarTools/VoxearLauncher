//! Fake signed engine releases, for tests and for trying the launcher before real releases exist. Each fake
//! engine has a `bin/vox` shell script that answers the `vox new` and `vox upgrade` contracts, so those flows
//! can be tried on Linux too. Enabled by the `testing` feature.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use semver::Version;

use crate::channel::Channel;
use crate::engine::{ENGINE_FILE, EngineInfo, Supports};
use crate::manifest::{Download, FORMAT, MANIFEST_FILE, Release, ReleaseManifest, SIGNATURE_SUFFIX};
use crate::platform::Platform;
use crate::signing::TrustedKeys;
use crate::store::hex;

/// A throwaway signing key.
pub struct TestKey {
    pair: minisign::KeyPair,
}

impl TestKey {
    pub fn generate() -> Self {
        Self { pair: minisign::KeyPair::generate_unencrypted_keypair().expect("the system has randomness") }
    }

    /// The public key's base64 line, as it would go in `trusted_keys`.
    pub fn public(&self) -> String {
        self.pair.pk.to_base64()
    }

    pub fn sign(&self, data: &[u8]) -> String {
        minisign::sign(Some(&self.pair.pk), &self.pair.sk, data, Some("voxear test release"), None)
            .expect("signing in memory can't fail")
            .into_string()
    }
}

pub struct ArchiveEntry {
    path: String,
    kind: EntryKind,
    data: Vec<u8>,
    mode: u32,
}

enum EntryKind {
    File,
    Dir,
    Symlink(String),
}

impl ArchiveEntry {
    pub fn file(path: impl Into<String>, data: &[u8], mode: u32) -> Self {
        Self { path: path.into(), kind: EntryKind::File, data: data.to_vec(), mode }
    }

    pub fn dir(path: impl Into<String>) -> Self {
        Self { path: path.into(), kind: EntryKind::Dir, data: Vec::new(), mode: 0o755 }
    }

    pub fn symlink(path: impl Into<String>, target: impl Into<String>) -> Self {
        Self { path: path.into(), kind: EntryKind::Symlink(target.into()), data: Vec::new(), mode: 0o777 }
    }
}

/// Builds a `.tar.zst`. Names are written as given, even ones a careful archiver would refuse, so tests can
/// build hostile archives.
pub fn tar_zst(entries: &[ArchiveEntry]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        write_field(&mut header.as_old_mut().name, &entry.path);
        header.set_mode(entry.mode);
        header.set_mtime(0);
        match &entry.kind {
            EntryKind::File => header.set_entry_type(tar::EntryType::Regular),
            EntryKind::Dir => header.set_entry_type(tar::EntryType::Directory),
            EntryKind::Symlink(target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                write_field(&mut header.as_old_mut().linkname, target);
            }
        }
        header.set_size(entry.data.len() as u64);
        header.set_cksum();
        builder.append(&header, entry.data.as_slice()).expect("writing to memory can't fail");
    }
    let tar = builder.into_inner().expect("writing to memory can't fail");
    zstd::encode_all(tar.as_slice(), 3).expect("compressing in memory can't fail")
}

fn write_field(field: &mut [u8], value: &str) {
    field.fill(0);
    field[..value.len()].copy_from_slice(value.as_bytes());
}

/// A fake engine install: `engine.toml`, a `bin/vox` script and a stand-in `Voxear.Core.dll`.
pub fn engine_archive(version: &Version, channel: Channel, platform: Platform) -> Vec<u8> {
    let info = EngineInfo {
        format: 1,
        version: version.clone(),
        channel,
        platform: platform.as_str().to_owned(),
        commit: Some("0000000".to_owned()),
        supports: Supports { new: Some(1), upgrade: Some(1) },
    };
    let info = toml::to_string(&info).expect("engine info serializes");
    let vox = FAKE_VOX.replace("{version}", &version.to_string());
    tar_zst(&[
        ArchiveEntry::file(ENGINE_FILE, info.as_bytes(), 0o644),
        ArchiveEntry::dir("bin"),
        ArchiveEntry::file(format!("bin/vox{}", platform.exe_suffix()), vox.as_bytes(), 0o755),
        ArchiveEntry::dir("managed"),
        ArchiveEntry::file("managed/Voxear.Core.dll", b"MZ", 0o644),
    ])
}

/// A folder of fake releases with a signed manifest, laid out the way a release mirror would be.
pub struct FakeReleases {
    pub dir: PathBuf,
    pub key: TestKey,
    pub manifest: ReleaseManifest,
}

impl FakeReleases {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).expect("the folder can be created");
        let manifest = ReleaseManifest { format: FORMAT, generated: Timestamp::now(), releases: Vec::new() };
        Self { dir, key: TestKey::generate(), manifest }
    }

    pub fn trusted_keys(&self) -> TrustedKeys {
        let mut keys = TrustedKeys::new();
        keys.add(&self.key.public()).expect("test keys are valid");
        keys
    }

    /// Adds a release with a working fake engine for every platform.
    pub fn add(&mut self, version: &str, channel: Channel) -> &mut Release {
        let version = Version::parse(version).expect("test versions are valid");
        for platform in Platform::ALL {
            let archive = engine_archive(&version, channel, platform);
            self.add_archive(&version, channel, platform, &archive);
        }
        self.release(&version)
    }

    /// Adds `archive` as the download of `version` for `platform`, creating the release if needed.
    pub fn add_archive(&mut self, version: &Version, channel: Channel, platform: Platform, archive: &[u8]) {
        let file = format!("voxear-{version}-{platform}.tar.zst");
        let signature = self.key.sign(archive);
        std::fs::write(self.dir.join(&file), archive).expect("the archive can be written");
        write_signature(&self.dir.join(&file), &signature);
        let download = Download {
            platform: platform.as_str().to_owned(),
            url: file,
            size: archive.len() as u64,
            sha256: hex(&<sha2::Sha256 as sha2::Digest>::digest(archive)),
            signature,
        };
        if self.manifest.find(version).is_none() {
            self.manifest.releases.push(Release {
                version: version.clone(),
                channel,
                published: Timestamp::now(),
                notes: None,
                min_launcher: None,
                yanked: false,
                downloads: Vec::new(),
            });
        }
        let release = self.release(version);
        release.downloads.retain(|existing| existing.platform != platform.as_str());
        release.downloads.push(download);
    }

    pub fn release(&mut self, version: &Version) -> &mut Release {
        self.manifest.releases.iter_mut().find(|release| &release.version == version).expect("the release exists")
    }

    pub fn archive_path(&self, version: &str, platform: Platform) -> PathBuf {
        self.dir.join(format!("voxear-{version}-{platform}.tar.zst"))
    }

    pub fn url(&self) -> String {
        self.dir.join(MANIFEST_FILE).to_string_lossy().into_owned()
    }

    /// Writes the manifest and its signature, and returns where the manifest is.
    pub fn write(&self) -> String {
        let text = self.manifest.to_toml().expect("the manifest serializes");
        let path = self.dir.join(MANIFEST_FILE);
        std::fs::write(&path, &text).expect("the manifest can be written");
        write_signature(&path, &self.key.sign(text.as_bytes()));
        self.url()
    }
}

fn write_signature(path: &Path, signature: &str) {
    let mut name = path.as_os_str().to_owned();
    name.push(SIGNATURE_SUFFIX);
    std::fs::write(PathBuf::from(name), signature).expect("the signature can be written");
}

/// The fake engine's `vox`: just enough of the `vox new` contract to try the launcher with.
const FAKE_VOX: &str = r#"#!/bin/sh
# A stand-in for `vox` in a fake Voxear release. It answers the launcher's `vox new` contract.
set -e
if [ "$1" = "--version" ]; then echo "vox {version}"; exit 0; fi
if [ "$1" = "upgrade" ]; then
  shift
  path="$1"; shift
  from="an unpinned project"
  while [ $# -gt 0 ]; do
    case "$1" in
      --from) from="Voxear $2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [ -e "$path/FAIL_UPGRADE" ]; then
    echo '{"event":"error","message":"this fake engine was asked to fail"}'
    exit 1
  fi
  echo '{"event":"step","message":"Reading voxear.toml"}'
  if grep -q '^engine = ' "$path/voxear.toml"; then
    sed -i 's/^engine = .*/engine = "{version}"/' "$path/voxear.toml"
  else
    sed -i 's/^\[project\]$/[project]\nengine = "{version}"/' "$path/voxear.toml"
  fi
  echo "{\"event\":\"change\",\"message\":\"Moved from $from to Voxear {version}\"}"
  echo '{"event":"change","message":"Converted 2 scenes to the new scene format"}'
  echo '{"event":"done"}'
  exit 0
fi
if [ "$1" != "new" ]; then echo "this fake vox only knows \`vox new\` and \`vox upgrade\`" >&2; exit 2; fi
shift
if [ "$1" = "--describe" ]; then
cat <<'JSON'
{"format":1,
 "templates":[
  {"id":"empty","name":"Empty","description":"Nothing but a camera","setup":["camera"]},
  {"id":"3d","name":"3D","description":"A lit 3D scene","setup":["lighting","camera","input","sample-scene","quality"]},
  {"id":"2d","name":"2D","description":"A 2D scene with a pixel-perfect camera","setup":["camera","input","sample-scene"]},
  {"id":"first-person","name":"First-person","description":"A first-person controller","setup":["lighting","camera","input","sample-scene","quality"]},
  {"id":"third-person","name":"Third-person","description":"A third-person controller and camera rig","setup":["lighting","camera","input","sample-scene","quality"]},
  {"id":"top-down","name":"Top-down","description":"A top-down camera and click-to-move","setup":["lighting","camera","input","sample-scene","quality"]}],
 "setup":[
  {"id":"lighting","name":"Lighting and sky"},
  {"id":"camera","name":"Camera"},
  {"id":"input","name":"Input actions"},
  {"id":"sample-scene","name":"Sample scene"},
  {"id":"quality","name":"Quality presets"}],
 "ides":[
  {"id":"rider","name":"Rider"},
  {"id":"visual-studio","name":"Visual Studio"},
  {"id":"vscode","name":"VS Code"}]}
JSON
exit 0
fi
path="$1"; shift
name=""; template="empty"
while [ $# -gt 0 ]; do
  case "$1" in
    --name) name="$2"; shift 2 ;;
    --template) template="$2"; shift 2 ;;
    --setup|--ide|--steam-app-id) shift 2 ;;
    *) shift ;;
  esac
done
echo "{\"event\":\"step\",\"message\":\"Copying the $template template\"}"
mkdir -p "$path"
printf '[project]\nname = "%s"\nengine = "%s"\n' "$name" "{version}" > "$path/voxear.toml"
echo "{\"event\":\"step\",\"message\":\"Writing voxear.toml\"}"
echo "{\"event\":\"done\",\"path\":\"$path\"}"
"#;
