//! What an engine install says about itself. Every engine release has an `engine.toml` at its root, which is
//! how the launcher knows the version of an archive or of a folder it is pointed at.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::de::DeserializeOwned;
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
    /// Which launcher contracts this install's `vox` implements.
    #[serde(default)]
    pub supports: Supports,
}

/// The launcher contracts an install implements, each with the format it speaks. A missing entry means the
/// install doesn't have that command, so the launcher never has to run something to find out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Supports {
    /// `vox new`, and the format of `vox new --describe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<u32>,
    /// `vox upgrade`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade: Option<u32>,
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

/// How an engine command ended.
pub struct Finished {
    pub success: bool,
    /// Everything it wrote to stderr: its log, and the reason when it failed.
    pub stderr: String,
}

/// Runs an engine command that reports what it's doing as one JSON object per line on stdout (`--json`), and
/// passes each line it understands to `on_event`. Lines it doesn't understand are skipped, so the engine can add
/// events without breaking older launchers.
pub fn run_events<E: DeserializeOwned>(command: &mut Command, on_event: &mut dyn FnMut(E)) -> Result<Finished> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("couldn't run {program}"))?;

    // Read stderr on its own thread, so a chatty engine can't fill the pipe and stall.
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr = std::thread::spawn(move || {
        let mut text = String::new();
        _ = stderr.read_to_string(&mut text);
        text
    });

    let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    for line in stdout.lines() {
        if let Ok(event) =
            serde_json::from_str::<E>(&line.with_context(|| format!("couldn't read {program}'s output"))?)
        {
            on_event(event);
        }
    }
    let status = child.wait().with_context(|| format!("couldn't wait for {program}"))?;
    Ok(Finished { success: status.success(), stderr: stderr.join().unwrap_or_default() })
}
