//! Launcher settings (`settings.toml`).

use std::io::ErrorKind;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::layout::Layout;
use crate::signing::TrustedKeys;

/// Where official engine releases are listed. Unset until release hosting is decided (see the open questions
/// in `docs/DESIGN.md`); until then, set `releases` in the settings or `VOXUP_RELEASES`.
pub const DEFAULT_RELEASES: Option<&str> = None;
/// Overrides where engine releases are listed, for mirrors and testing.
pub const RELEASES_VAR: &str = "VOXUP_RELEASES";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Where engine releases are listed: an `https://` URL or a local path to a `releases.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub releases: Option<String>,
    /// Keys to trust for engine releases besides the built-in ones, for self-hosted engine builds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_keys: Vec<String>,
    /// The folder new projects go in, remembered from the last one created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_location: Option<String>,
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

    /// `VOXUP_RELEASES`, then the setting, then the official location.
    pub fn releases_url(&self) -> Option<String> {
        std::env::var(RELEASES_VAR)
            .ok()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| self.releases.clone())
            .or_else(|| DEFAULT_RELEASES.map(str::to_owned))
    }

    pub fn trusted_keys(&self) -> Result<TrustedKeys> {
        TrustedKeys::engine_releases(self.trusted_keys.iter().map(String::as_str))
            .context("a key in trusted_keys in the settings isn't valid")
    }
}
