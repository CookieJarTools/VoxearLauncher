//! The engine release manifest (`releases.toml`): every published engine version, its channel and its
//! downloads. The manifest is signed as a whole, and each download carries its own signature as well, so an
//! archive can also be checked on its own.

use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};
use jiff::Timestamp;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::channel::{Channel, Selector};
use crate::platform::Platform;

/// The manifest format this launcher reads. A manifest with a higher number needs a newer launcher.
pub const FORMAT: u32 = 1;
pub const MANIFEST_FILE: &str = "releases.toml";
/// Detached signatures sit next to what they sign, under the same name plus this.
pub const SIGNATURE_SUFFIX: &str = ".minisig";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub format: u32,
    /// When the manifest was written. The launcher refuses a manifest older than one it has already seen, so
    /// nobody can quietly serve an old one to hide a newer release.
    pub generated: Timestamp,
    #[serde(default, rename = "release")]
    pub releases: Vec<Release>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub version: Version,
    pub channel: Channel,
    pub published: Timestamp,
    /// A link to the release notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The oldest launcher that can install this release, when it needs something newer launchers know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_launcher: Option<Version>,
    /// Pulled after publishing. Never picked for a channel or range, but still installable by exact version,
    /// so projects pinned to it keep opening.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub yanked: bool,
    #[serde(default, rename = "download")]
    pub downloads: Vec<Download>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Download {
    /// For example `linux-x86_64`. Kept as text so platforms this launcher doesn't know still parse.
    pub platform: String,
    /// Absolute, or relative to the manifest's own URL.
    pub url: String,
    /// The archive's size in bytes.
    pub size: u64,
    /// The archive's SHA-256, in hex.
    pub sha256: String,
    /// The archive's minisign signature: the whole `.minisig` file.
    pub signature: String,
}

impl ReleaseManifest {
    /// Parses and checks a manifest. Its signature must already have been checked.
    pub fn parse(text: &str) -> Result<Self> {
        let header: FormatOnly = toml::from_str(text).context("the release manifest isn't valid TOML")?;
        ensure!(
            header.format <= FORMAT,
            "the release manifest uses format {}, but this launcher reads format {FORMAT}; update the launcher",
            header.format
        );
        let manifest: ReleaseManifest = toml::from_str(text).context("the release manifest isn't valid")?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string(self)?)
    }

    fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for release in &self.releases {
            ensure!(seen.insert(&release.version), "the release manifest lists {} twice", release.version);
            ensure!(
                release.channel.accepts(&release.version),
                "the release manifest puts {} in the {} channel, which doesn't fit its version",
                release.version,
                release.channel
            );
            release.validate()?;
        }
        Ok(())
    }

    pub fn find(&self, version: &Version) -> Option<&Release> {
        self.releases.iter().find(|release| &release.version == version)
    }

    /// Releases that can be installed on `platform`, newest first. Yanked releases are left out.
    pub fn available(&self, platform: Platform) -> Vec<&Release> {
        let mut releases: Vec<_> =
            self.releases.iter().filter(|release| !release.yanked && release.download(platform).is_some()).collect();
        releases.sort_by(|a, b| b.version.cmp(&a.version));
        releases
    }

    /// The release `selector` picks for `platform`.
    pub fn select(&self, selector: &Selector, platform: Platform) -> Option<&Release> {
        if let Selector::Exact(version) = selector {
            return self.find(version).filter(|release| release.download(platform).is_some());
        }
        self.available(platform).into_iter().find(|release| match selector {
            Selector::Latest(channel) => release.channel == *channel,
            Selector::Matching(req) => req.matches(&release.version),
            Selector::Exact(_) => unreachable!(),
        })
    }
}

impl ReleaseManifest {
    /// The releases a project passes through on its way from `from` to `to`, oldest first: the stable releases
    /// after `from`, up to `to`, and `to` itself. Moving to an older version passes through none.
    pub fn between(&self, from: Option<&Version>, to: &Version) -> Vec<&Release> {
        let mut releases: Vec<_> = self
            .releases
            .iter()
            .filter(|release| {
                let after_from = from.map_or(&release.version == to, |from| &release.version > from);
                after_from && &release.version <= to && (release.version.pre.is_empty() || &release.version == to)
            })
            .collect();
        releases.sort_by(|a, b| a.version.cmp(&b.version));
        releases
    }
}

impl Release {
    pub fn download(&self, platform: Platform) -> Option<&Download> {
        self.downloads.iter().find(|download| download.platform == platform.as_str())
    }

    /// Whether this launcher is too old to install the release.
    pub fn needs_newer_launcher(&self) -> bool {
        self.min_launcher.as_ref().is_some_and(|min| &crate::launcher_version() < min)
    }

    fn validate(&self) -> Result<()> {
        let mut platforms = HashSet::new();
        for download in &self.downloads {
            ensure!(
                platforms.insert(download.platform.as_str()),
                "the release manifest has two {} downloads for {}",
                download.platform,
                self.version
            );
            ensure!(
                !download.url.trim().is_empty(),
                "the {} download for {} has no URL",
                download.platform,
                self.version
            );
            if !is_sha256(&download.sha256) {
                bail!("the {} download for {} has a malformed SHA-256", download.platform, self.version);
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

pub fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    fn manifest(releases: &str) -> String {
        format!("format = 1\ngenerated = \"2026-10-30T12:00:00Z\"\n{releases}")
    }

    fn release(version: &str, channel: &str, extra: &str) -> String {
        format!(
            r#"
[[release]]
version = "{version}"
channel = "{channel}"
published = "2026-10-30T12:00:00Z"
{extra}

[[release.download]]
platform = "linux-x86_64"
url = "voxear-{version}-linux-x86_64.tar.zst"
size = 1024
sha256 = "{HASH}"
signature = "sig"

[[release.download]]
platform = "windows-x86_64"
url = "https://example.invalid/voxear-{version}-windows-x86_64.tar.zst"
size = 1024
sha256 = "{HASH}"
signature = "sig"
"#
        )
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn sample() -> ReleaseManifest {
        let text = manifest(
            &[
                release("0.2.0", "stable", ""),
                release("0.3.0", "stable", ""),
                release("0.3.1", "stable", "yanked = true"),
                release("0.4.0-beta.1", "beta", ""),
                release("0.4.0-beta.2", "beta", ""),
                release("0.4.0-nightly.20261030", "nightly", ""),
            ]
            .concat(),
        );
        ReleaseManifest::parse(&text).unwrap()
    }

    #[test]
    fn parses_releases_and_downloads() {
        let manifest = sample();
        assert_eq!(manifest.releases.len(), 6);
        let release = manifest.find(&v("0.3.0")).unwrap();
        assert_eq!(release.channel, Channel::Stable);
        let download = release.download(Platform::LinuxX86_64).unwrap();
        assert_eq!(download.size, 1024);
        assert_eq!(download.url, "voxear-0.3.0-linux-x86_64.tar.zst");
    }

    #[test]
    fn picks_the_newest_release_in_a_channel_skipping_yanked_ones() {
        let manifest = sample();
        let pick = |s: &str| manifest.select(&s.parse().unwrap(), Platform::LinuxX86_64).map(|r| r.version.to_string());
        assert_eq!(pick("stable").as_deref(), Some("0.3.0"));
        assert_eq!(pick("beta").as_deref(), Some("0.4.0-beta.2"));
        assert_eq!(pick("nightly").as_deref(), Some("0.4.0-nightly.20261030"));
        assert_eq!(pick("0.2").as_deref(), Some("0.2.0"));
        assert_eq!(pick("0.3").as_deref(), Some("0.3.0"));
        assert_eq!(pick("0.3.1").as_deref(), Some("0.3.1"), "exact versions still find yanked releases");
        assert_eq!(pick("9.9.9"), None);
    }

    #[test]
    fn lists_available_releases_newest_first() {
        let manifest = sample();
        let versions: Vec<_> =
            manifest.available(Platform::WindowsX86_64).iter().map(|r| r.version.to_string()).collect();
        assert_eq!(versions, ["0.4.0-nightly.20261030", "0.4.0-beta.2", "0.4.0-beta.1", "0.3.0", "0.2.0"]);
    }

    #[test]
    fn ignores_platforms_and_fields_it_does_not_know() {
        let text =
            manifest(&release("0.3.0", "stable", "future_field = true")).replace("windows-x86_64", "linux-riscv64");
        let manifest = ReleaseManifest::parse(&text).unwrap();
        let release = manifest.find(&v("0.3.0")).unwrap();
        assert!(release.download(Platform::WindowsX86_64).is_none());
        assert!(release.download(Platform::LinuxX86_64).is_some());
    }

    #[test]
    fn rejects_a_newer_format() {
        let text = manifest("").replace("format = 1", "format = 2");
        let error = ReleaseManifest::parse(&text).unwrap_err().to_string();
        assert!(error.contains("update the launcher"), "{error}");
    }

    #[test]
    fn rejects_a_version_in_the_wrong_channel() {
        let error = ReleaseManifest::parse(&manifest(&release("0.4.0-beta.1", "stable", ""))).unwrap_err();
        assert!(error.to_string().contains("doesn't fit its version"), "{error}");
    }

    #[test]
    fn rejects_duplicate_versions_and_bad_hashes() {
        let twice = [release("0.3.0", "stable", ""), release("0.3.0", "stable", "")].concat();
        assert!(ReleaseManifest::parse(&manifest(&twice)).is_err());
        let bad_hash = manifest(&release("0.3.0", "stable", "")).replace(HASH, "not-a-hash");
        assert!(ReleaseManifest::parse(&bad_hash).is_err());
    }

    #[test]
    fn flags_releases_that_need_a_newer_launcher() {
        let text = manifest(&release("0.3.0", "stable", "min_launcher = \"99.0.0\""));
        let manifest = ReleaseManifest::parse(&text).unwrap();
        assert!(manifest.find(&v("0.3.0")).unwrap().needs_newer_launcher());
        assert!(!sample().find(&v("0.3.0")).unwrap().needs_newer_launcher());
    }

    #[test]
    fn lists_the_releases_between_two_versions() {
        let manifest = sample();
        let between = |from: Option<&str>, to: &str| {
            let from = from.map(v);
            manifest.between(from.as_ref(), &v(to)).iter().map(|r| r.version.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(between(Some("0.2.0"), "0.3.1"), ["0.3.0", "0.3.1"]);
        assert_eq!(between(Some("0.2.0"), "0.4.0-beta.2"), ["0.3.0", "0.3.1", "0.4.0-beta.2"]);
        assert_eq!(between(None, "0.3.0"), ["0.3.0"]);
        assert!(between(Some("0.3.0"), "0.2.0").is_empty());
    }

    #[test]
    fn round_trips_through_toml() {
        let manifest = sample();
        let again = ReleaseManifest::parse(&manifest.to_toml().unwrap()).unwrap();
        assert_eq!(again.releases.len(), manifest.releases.len());
        assert!(again.find(&v("0.3.1")).unwrap().yanked);
    }
}
