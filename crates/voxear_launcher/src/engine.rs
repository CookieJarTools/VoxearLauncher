//! What an engine install says about itself. Every engine release has an `engine.toml` at its root, which is
//! how the launcher knows the version of an archive or of a folder it is pointed at.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::channel::Channel;

pub const ENGINE_FILE: &str = "engine.toml";
pub const FORMAT: u32 = 1;
/// Set for every engine process the launcher starts, so `vox` and the editor find their own files.
pub const HOME_VAR: &str = "VOXEAR_HOME";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub format: u32,
    pub version: Version,
    pub channel: Channel,
    pub platform: String,
    /// The engine commit it was built from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

impl EngineInfo {
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join(ENGINE_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("{} isn't a Voxear engine (it has no {ENGINE_FILE})", dir.display()))?;
        let info: EngineInfo = toml::from_str(&text).with_context(|| format!("{} isn't valid", path.display()))?;
        ensure!(
            info.format <= FORMAT,
            "{} uses format {}, but this launcher reads format {FORMAT}; update the launcher",
            path.display(),
            info.format
        );
        Ok(info)
    }
}

/// The files the launcher uses inside an engine install.
pub fn vox(home: &Path) -> PathBuf {
    home.join("bin").join(format!("vox{}", std::env::consts::EXE_SUFFIX))
}

pub fn editor(home: &Path) -> PathBuf {
    home.join("bin").join(format!("voxear-editor{}", std::env::consts::EXE_SUFFIX))
}

/// A command for a program inside the install at `home`, with `VOXEAR_HOME` pointing at it.
pub fn command(home: &Path, program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env(HOME_VAR, home);
    command
}
