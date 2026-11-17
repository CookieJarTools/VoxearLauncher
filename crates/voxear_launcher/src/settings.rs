//! Launcher settings (`settings.toml`).

use std::io::ErrorKind;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::fsutil;
use crate::layout::Layout;
use crate::signing::TrustedKeys;
use crate::sources::{self, OFFICIAL, Source};
use crate::store::Install;

/// Where official engine releases are listed: GitHub Pages for the public `CookieJarTools/VoxearReleases`
/// repository. The archives themselves are that repository's release assets.
pub const OFFICIAL_RELEASES: &str = "https://cookiejartools.github.io/VoxearReleases/engine/releases.toml";
/// Overrides where official engine releases are listed, for mirrors and testing.
pub const RELEASES_VAR: &str = "VOXUP_RELEASES";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "SavedSettings")]
pub struct Settings {
    /// Whether nightly builds show in the engine list. Unset, they show once a nightly is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nightlies: Option<bool>,
    /// Keys to trust for engine releases besides the built-in ones, such as for archives installed from a file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_keys: Vec<String>,
    /// The folder new projects go in, remembered from the last one created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_location: Option<String>,
    /// Which launcher updates to take. Stable unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_channel: Option<crate::self_update::UpdateChannel>,
    /// Engine releases someone else publishes, such as a fork of the engine, listed beside the official ones.
    /// Last, so the file keeps its plain values above these tables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_releases: Vec<CustomReleases>,
}

/// Someone else's engine releases, published the way the official ones are: a signed `releases.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomReleases {
    /// What the engine list calls them, such as the fork's name.
    pub name: String,
    /// Where their releases are listed: their `releases.toml` (an `https://` URL or a file), the site or folder it
    /// is in, or the GitHub repository it is published from (`owner/repo`).
    pub url: String,
    /// The public key their releases are signed with, unless it is in `trusted_keys` already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// The file as saved, including settings that have since been replaced.
#[derive(Deserialize)]
struct SavedSettings {
    #[serde(default)]
    nightlies: Option<bool>,
    #[serde(default)]
    trusted_keys: Vec<String>,
    #[serde(default)]
    project_location: Option<String>,
    #[serde(default)]
    launcher_channel: Option<crate::self_update::UpdateChannel>,
    #[serde(default)]
    custom_releases: Vec<CustomReleases>,
    /// From before engine releases were published: one release list that took the place of the official one.
    #[serde(default)]
    releases: Option<String>,
}

impl From<SavedSettings> for Settings {
    fn from(saved: SavedSettings) -> Self {
        let mut settings = Settings {
            nightlies: saved.nightlies,
            trusted_keys: saved.trusted_keys,
            project_location: saved.project_location,
            launcher_channel: saved.launcher_channel,
            custom_releases: saved.custom_releases,
        };
        if let Some(old) = saved.releases {
            settings.move_old_releases(&old);
        }
        settings
    }
}

impl Settings {
    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.settings_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("{} isn't valid", path.display())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("couldn't read {}", path.display())),
        }
    }

    pub fn save(&self, layout: &Layout) -> Result<()> {
        fsutil::write_atomic(&layout.settings_file(), toml::to_string(self)?.as_bytes())
    }

    /// The old `releases` setting pointed the launcher at one release list instead of the official one, back when
    /// nothing official was published. Anything but the official list itself moves over as custom releases, so
    /// it is still listed, now beside the official releases.
    fn move_old_releases(&mut self, old: &str) {
        let old = old.trim();
        if old.is_empty() || sources::same_place(old, OFFICIAL_RELEASES) {
            return;
        }
        if self.custom_releases.iter().any(|custom| sources::same_place(&custom.url, old)) {
            return;
        }
        let name = self.free_name(&sources::name_for(old));
        self.custom_releases.push(CustomReleases { name, url: old.to_owned(), key: None });
    }

    /// `name`, or `name 2`, `name 3` and so on when it is taken.
    fn free_name(&self, name: &str) -> String {
        let taken = |candidate: &str| candidate.eq_ignore_ascii_case(OFFICIAL) || self.custom(candidate).is_some();
        if !taken(name) {
            return name.to_owned();
        }
        (2..).map(|n| format!("{name} {n}")).find(|candidate| !taken(candidate)).expect("some number is free")
    }

    /// The custom releases called `name`, ignoring case.
    pub fn custom(&self, name: &str) -> Option<&CustomReleases> {
        self.custom_releases.iter().find(|custom| custom.name.trim().eq_ignore_ascii_case(name.trim()))
    }

    /// Adds someone else's releases. `url` may be any address [`sources::manifest_address`] understands; it is
    /// saved as the manifest's own address. The name defaults to one made from the address.
    pub fn add_custom(&mut self, url: &str, name: Option<&str>, key: Option<&str>) -> Result<&CustomReleases> {
        let url = sources::manifest_address(url)?;
        let url =
            if url.contains("://") { url } else { fsutil::absolute(url.as_ref())?.to_string_lossy().into_owned() };
        ensure!(
            !sources::same_place(&url, OFFICIAL_RELEASES),
            "those are the official releases, which are always listed"
        );
        if let Some(existing) = self.custom_releases.iter().find(|custom| sources::same_place(&custom.url, &url)) {
            bail!("those releases are already listed, as {}", existing.name);
        }
        let name = match name.map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => {
                ensure!(!name.eq_ignore_ascii_case(OFFICIAL), "\"{OFFICIAL}\" is the official releases' name");
                ensure!(self.custom(name).is_none(), "there are custom releases called {name} already");
                name.to_owned()
            }
            None => self.free_name(&sources::name_for(&url)),
        };
        let key = key.map(str::trim).filter(|key| !key.is_empty()).map(str::to_owned);
        if let Some(key) = &key {
            TrustedKeys::new().add(key).context("the key isn't valid")?;
        }
        self.custom_releases.push(CustomReleases { name, url, key });
        Ok(self.custom_releases.last().expect("just added"))
    }

    /// Stops listing the custom releases called `name`. Engines installed from them stay.
    pub fn remove_custom(&mut self, name: &str) -> Result<CustomReleases> {
        let index = self
            .custom_releases
            .iter()
            .position(|custom| custom.name.trim().eq_ignore_ascii_case(name.trim()))
            .with_context(|| format!("there are no custom releases called {name}"))?;
        Ok(self.custom_releases.remove(index))
    }

    /// Checks what a settings file or the window would save: every key parses, and custom releases have names of
    /// their own and addresses the launcher can read.
    pub fn check(&self) -> Result<()> {
        self.trusted_keys()?;
        for (index, custom) in self.custom_releases.iter().enumerate() {
            let name = custom.name.trim();
            ensure!(!name.is_empty(), "custom releases need a name");
            ensure!(!name.eq_ignore_ascii_case(OFFICIAL), "\"{OFFICIAL}\" is the official releases' name");
            ensure!(
                !self.custom_releases[..index].iter().any(|other| other.name.trim().eq_ignore_ascii_case(name)),
                "two custom releases are called {name}"
            );
            sources::manifest_address(&custom.url).with_context(|| format!("the address of {name} isn't valid"))?;
            if let Some(key) = &custom.key {
                TrustedKeys::new().add(key).with_context(|| format!("the key of {name} isn't valid"))?;
            }
        }
        Ok(())
    }

    /// Whether nightly builds show in the engine list: as set, or, when it never was, whether one is installed.
    pub fn nightlies_shown(&self, installs: &[Install]) -> bool {
        self.nightlies.unwrap_or_else(|| installs.iter().any(|install| install.channel == Channel::Nightly))
    }

    /// The built-in engine release keys and `trusted_keys`.
    pub fn trusted_keys(&self) -> Result<TrustedKeys> {
        TrustedKeys::engine_releases(self.trusted_keys.iter().map(String::as_str))
            .context("a key in trusted_keys in the settings isn't valid")
    }

    /// Every release list: the official one first, then the custom ones in order.
    pub fn sources(&self) -> Result<Vec<Source>> {
        self.sources_with(&official_url())
    }

    /// The same, with the official releases listed at `official`.
    pub fn sources_with(&self, official: &str) -> Result<Vec<Source>> {
        let keys = self.trusted_keys()?;
        let mut sources = vec![Source { name: OFFICIAL.to_owned(), url: official.to_owned(), official: true, keys }];
        for custom in &self.custom_releases {
            let mut keys = self.trusted_keys()?;
            if let Some(key) = &custom.key {
                keys.add(key).with_context(|| format!("the key of {} isn't valid", custom.name))?;
            }
            let url = sources::manifest_address(&custom.url)
                .with_context(|| format!("the address of {} isn't valid", custom.name))?;
            sources.push(Source { name: custom.name.clone(), url, official: false, keys });
        }
        Ok(sources)
    }

    /// What to call the release list at `url`, which an engine was installed from: nothing for the official one,
    /// listed at `official` or its usual place, the name of custom releases, or a name made from the address for a
    /// list no longer in the settings.
    pub fn source_name(&self, url: &str, official: &str) -> Option<String> {
        if sources::same_place(url, official) || sources::same_place(url, OFFICIAL_RELEASES) {
            return None;
        }
        let custom = self.custom_releases.iter().find(|custom| sources::same_place(&custom.url, url));
        Some(custom.map_or_else(|| sources::name_for(url), |custom| custom.name.clone()))
    }
}

/// Where the official releases are listed: `VOXUP_RELEASES` when it is set, otherwise their usual place.
pub fn official_url() -> String {
    std::env::var(RELEASES_VAR)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| OFFICIAL_RELEASES.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Settings {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn moves_an_old_release_list_over_as_custom_releases() {
        let settings = parse("releases = \"/srv/voxear-mirror/releases.toml\"\ntrusted_keys = [\"RWQ\"]\n");
        let moved = CustomReleases {
            name: "voxear-mirror".to_owned(),
            url: "/srv/voxear-mirror/releases.toml".to_owned(),
            key: None,
        };
        assert_eq!(settings.custom_releases, [moved]);
        assert_eq!(settings.trusted_keys, ["RWQ"], "its key stays trusted");

        let saved = toml::to_string(&settings).unwrap();
        assert!(
            !saved.contains("\nreleases = ") && !saved.starts_with("releases = "),
            "the old setting is gone:\n{saved}"
        );
        assert_eq!(parse(&saved), settings);
    }

    #[test]
    fn names_moved_releases_after_where_they_are() {
        let named = |old: &str| parse(&format!("releases = {old:?}")).custom_releases[0].name.clone();
        assert_eq!(named("https://someone.github.io/VoxearReleases/engine/releases.toml"), "someone");
        assert_eq!(named("https://mirror.example.org/voxear/releases.toml"), "mirror.example.org");
        assert_eq!(named("/srv/site/engine/releases.toml"), "site");
    }

    #[test]
    fn drops_an_old_setting_that_named_the_official_releases() {
        let official = [
            OFFICIAL_RELEASES,
            "https://cookiejartools.github.io/VoxearReleases/",
            "CookieJarTools/VoxearReleases",
            "",
        ];
        for old in official {
            let settings = parse(&format!("releases = {old:?}"));
            assert!(settings.custom_releases.is_empty(), "{old}");
        }
    }

    #[test]
    fn does_not_list_moved_releases_twice() {
        let text = "releases = \"https://fork.example.org/releases.toml\"\n\n[[custom_releases]]\nname = \"Fork\"\n\
                    url = \"https://fork.example.org/releases.toml\"\n";
        let settings = parse(text);
        assert_eq!(settings.custom_releases.len(), 1);
        assert_eq!(settings.custom_releases[0].name, "Fork");
    }

    #[test]
    fn gives_moved_releases_a_name_of_their_own() {
        let text = "releases = \"https://fork.example.org/a/releases.toml\"\n\n[[custom_releases]]\n\
                    name = \"fork.example.org\"\nurl = \"https://fork.example.org/b/releases.toml\"\n";
        let names: Vec<_> = parse(text).custom_releases.into_iter().map(|custom| custom.name).collect();
        assert_eq!(names, ["fork.example.org", "fork.example.org 2"]);
    }

    #[test]
    fn adds_and_removes_custom_releases() {
        let mut settings = Settings::default();
        let key = crate::testing::TestKey::generate().public();
        let added = settings.add_custom("someone/VoxearReleases", None, Some(&key)).unwrap();
        assert_eq!(added.name, "someone");
        assert_eq!(added.url, "https://someone.github.io/VoxearReleases/engine/releases.toml");
        assert_eq!(added.key.as_deref(), Some(key.as_str()));

        let again = settings.add_custom("https://github.com/someone/VoxearReleases", Some("Again"), None);
        assert!(again.unwrap_err().to_string().contains("already listed, as someone"));
        assert!(settings.add_custom("https://other.example.org/", Some("SOMEONE"), None).is_err(), "names are unique");
        assert!(settings.add_custom("https://other.example.org/", Some("official"), None).is_err());
        assert!(settings.add_custom(OFFICIAL_RELEASES, None, None).is_err());
        assert!(settings.add_custom("https://other.example.org/", None, Some("not a key")).is_err());
        assert!(settings.add_custom("http://other.example.org/", None, None).is_err(), "plain HTTP is refused");
        settings.check().unwrap();

        assert_eq!(settings.remove_custom("Someone").unwrap().name, "someone");
        assert!(settings.custom_releases.is_empty());
        assert!(settings.remove_custom("someone").is_err());
    }

    #[test]
    fn checks_names_and_keys() {
        let custom = |name: &str, key: Option<&str>| CustomReleases {
            name: name.to_owned(),
            url: "https://fork.example.org/releases.toml".to_owned(),
            key: key.map(str::to_owned),
        };
        let with = |list: Vec<CustomReleases>| Settings { custom_releases: list, ..Settings::default() }.check();
        assert!(with(vec![custom("Fork", None)]).is_ok());
        assert!(with(vec![custom("Fork", None), custom("fork", None)]).is_err());
        assert!(with(vec![custom(" ", None)]).is_err());
        assert!(with(vec![custom("Official", None)]).is_err());
        assert!(with(vec![custom("Fork", Some("not a key"))]).is_err());
    }

    #[test]
    fn lists_the_official_releases_first() {
        let mut settings = Settings::default();
        settings.add_custom("https://fork.example.org/voxear/", Some("Fork"), None).unwrap();
        let sources = settings.sources_with(OFFICIAL_RELEASES).unwrap();
        let listed: Vec<_> = sources.iter().map(|source| (source.name.as_str(), source.official)).collect();
        assert_eq!(listed, [("Official", true), ("Fork", false)]);
        assert_eq!(sources[1].url, "https://fork.example.org/voxear/engine/releases.toml");
    }

    #[test]
    fn names_where_an_install_came_from() {
        let mut settings = Settings::default();
        settings.add_custom("https://fork.example.org/releases.toml", Some("Fork"), None).unwrap();
        let name = |url: &str| settings.source_name(url, "/srv/mirror/releases.toml");
        assert_eq!(name(OFFICIAL_RELEASES), None);
        assert_eq!(name("/srv/mirror/releases.toml"), None, "the official releases, listed elsewhere");
        assert_eq!(name("https://fork.example.org/releases.toml").as_deref(), Some("Fork"));
        assert_eq!(name("https://gone.example.org/releases.toml").as_deref(), Some("gone.example.org"));
    }

    #[test]
    fn shows_nightlies_once_one_is_installed_until_set() {
        let install = |version: &str, channel| Install {
            version: semver::Version::parse(version).unwrap(),
            channel,
            path: std::path::PathBuf::from("/engines").join(version),
            kind: crate::InstallKind::Managed,
            installed: None,
            missing: false,
            releases: None,
        };
        let stable = [install("0.1.0", Channel::Stable)];
        let nightly = [install("0.1.0", Channel::Stable), install("0.2.0-nightly.20261117", Channel::Nightly)];
        let unset = Settings::default();
        assert!(!unset.nightlies_shown(&stable));
        assert!(unset.nightlies_shown(&nightly));
        let off = Settings { nightlies: Some(false), ..Settings::default() };
        assert!(!off.nightlies_shown(&nightly));
        let on = Settings { nightlies: Some(true), ..Settings::default() };
        assert!(on.nightlies_shown(&stable));
    }
}
