//! The platforms engine releases are built for.

use std::fmt;
use std::str::FromStr;

use anyhow::bail;

/// A platform engine releases are built for. Manifests name platforms as strings (`linux-x86_64`), so a
/// manifest that also lists platforms this launcher doesn't know still parses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    LinuxX86_64,
    WindowsX86_64,
}

impl Platform {
    pub const ALL: [Platform; 2] = [Platform::LinuxX86_64, Platform::WindowsX86_64];

    /// The platform this launcher runs on, or `None` where no engine releases are built.
    pub fn current() -> Option<Platform> {
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Platform::LinuxX86_64)
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Platform::WindowsX86_64)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::LinuxX86_64 => "linux-x86_64",
            Platform::WindowsX86_64 => "windows-x86_64",
        }
    }

    pub fn exe_suffix(self) -> &'static str {
        match self {
            Platform::LinuxX86_64 => "",
            Platform::WindowsX86_64 => ".exe",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Platform {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match Platform::ALL.into_iter().find(|platform| platform.as_str() == s) {
            Some(platform) => Ok(platform),
            None => bail!("`{s}` is not a platform Voxear is built for (linux-x86_64, windows-x86_64)"),
        }
    }
}
