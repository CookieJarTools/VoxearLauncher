//! Where the launcher keeps engines and its own files.
//!
//! ```text
//! <root>/                    ~/.local/share/voxear, or %LOCALAPPDATA%\Voxear
//!   engines/<version>/       one folder per engine version: that version's VOXEAR_HOME
//!   located.toml             engines installed elsewhere that the launcher was pointed at
//!   projects.toml            the project list
//!   settings.toml            launcher settings
//!   backups/<project>/       project backups, made before a project changes engine version
//!   cache/                   the last verified release manifest, for offline use
//!   downloads/               archives while they download
//!   staging/                 archives while they unpack, renamed into engines/ when complete
//!   trash/                   engines being removed
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

/// Overrides the root folder, for portable setups and tests.
pub const HOME_VAR: &str = "VOXUP_HOME";

#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `VOXUP_HOME` if it is set, otherwise the platform's per-user data folder.
    pub fn from_env() -> Result<Self> {
        if let Some(root) = std::env::var_os(HOME_VAR).filter(|root| !root.is_empty()) {
            return Ok(Self::new(root));
        }
        let data = dirs::data_local_dir().ok_or_else(|| anyhow!("couldn't find this user's data folder"))?;
        Ok(Self::new(data.join(if cfg!(windows) { "Voxear" } else { "voxear" })))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn engines(&self) -> PathBuf {
        self.root.join("engines")
    }

    pub fn engine(&self, version: &semver::Version) -> PathBuf {
        self.engines().join(version.to_string())
    }

    pub fn located_file(&self) -> PathBuf {
        self.root.join("located.toml")
    }

    pub fn projects_file(&self) -> PathBuf {
        self.root.join("projects.toml")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }

    pub fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn trash(&self) -> PathBuf {
        self.root.join("trash")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("store.lock")
    }
}
