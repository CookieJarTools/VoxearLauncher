//! Release channels, and how a version is chosen from them.

use std::fmt;
use std::str::FromStr;

use anyhow::bail;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
    Nightly,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Stable, Channel::Beta, Channel::Nightly];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
            Channel::Nightly => "nightly",
        }
    }

    /// Whether a version's pre-release tag fits this channel. Stable versions have none (`0.3.0`), nightly
    /// ones start with `nightly` (`0.4.0-nightly.20261030`), and beta ones have any other tag (`0.4.0-beta.2`,
    /// `0.4.0-rc.1`).
    pub fn accepts(self, version: &Version) -> bool {
        let pre = version.pre.as_str();
        match self {
            Channel::Stable => pre.is_empty(),
            Channel::Nightly => pre.starts_with("nightly"),
            Channel::Beta => !pre.is_empty() && !pre.starts_with("nightly"),
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Channel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match Channel::ALL.into_iter().find(|channel| channel.as_str() == s) {
            Some(channel) => Ok(channel),
            None => bail!("`{s}` is not a channel (stable, beta or nightly)"),
        }
    }
}

/// What someone asked to install: `stable`, `0.3.1` or `0.3`.
#[derive(Clone, Debug, PartialEq)]
pub enum Selector {
    /// The newest release in a channel. `latest` means stable.
    Latest(Channel),
    /// Exactly this version, even if it was yanked.
    Exact(Version),
    /// The newest release matching a range. As with Cargo, `0.3` means `^0.3`, and pre-releases only match a
    /// range that names one.
    Matching(VersionReq),
}

impl FromStr for Selector {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let s = s.trim();
        if s == "latest" {
            return Ok(Selector::Latest(Channel::Stable));
        }
        if let Ok(channel) = s.parse() {
            return Ok(Selector::Latest(channel));
        }
        if let Ok(version) = Version::parse(s) {
            return Ok(Selector::Exact(version));
        }
        if let Ok(req) = VersionReq::parse(s) {
            return Ok(Selector::Matching(req));
        }
        bail!("`{s}` is not a version (0.3.1), a range (0.3) or a channel (stable, beta, nightly)")
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Selector::Latest(channel) => write!(f, "the latest {channel} release"),
            Selector::Exact(version) => write!(f, "{version}"),
            Selector::Matching(req) => write!(f, "{req}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn channels_accept_matching_pre_release_tags() {
        assert!(Channel::Stable.accepts(&v("0.3.0")));
        assert!(!Channel::Stable.accepts(&v("0.3.0-beta.1")));
        assert!(Channel::Beta.accepts(&v("0.4.0-beta.2")));
        assert!(Channel::Beta.accepts(&v("0.4.0-rc.1")));
        assert!(!Channel::Beta.accepts(&v("0.4.0")));
        assert!(!Channel::Beta.accepts(&v("0.4.0-nightly.20261030")));
        assert!(Channel::Nightly.accepts(&v("0.4.0-nightly.20261030")));
        assert!(!Channel::Nightly.accepts(&v("0.4.0-beta.1")));
    }

    #[test]
    fn selectors_parse_channels_versions_and_ranges() {
        assert_eq!("latest".parse::<Selector>().unwrap(), Selector::Latest(Channel::Stable));
        assert_eq!("nightly".parse::<Selector>().unwrap(), Selector::Latest(Channel::Nightly));
        assert_eq!("0.3.1".parse::<Selector>().unwrap(), Selector::Exact(v("0.3.1")));
        assert_eq!("0.3".parse::<Selector>().unwrap(), Selector::Matching(VersionReq::parse("0.3").unwrap()));
        assert!("banana".parse::<Selector>().is_err());
    }
}
