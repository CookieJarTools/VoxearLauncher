//! Small file helpers shared by the store and the registry.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Writes a file so that readers see either the old contents or the new ones, never half of each.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    let temp = unique_sibling(path, "tmp");
    let write = || -> std::io::Result<()> {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    };
    write().inspect_err(|_| _ = fs::remove_file(&temp)).with_context(|| format!("couldn't write {}", path.display()))
}

/// A path next to `path` that nothing else is using, for staging something before it is renamed into place.
pub fn unique_sibling(path: &Path, tag: &str) -> PathBuf {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{tag}-{}", unique_suffix()))
}

/// Unique within this machine for the life of the process: the process ID, the time and a counter.
pub fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("{}-{nanos:x}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Removes a folder when dropped, unless [`TempDir::keep`] was called. Used so a failed install never leaves
/// a half-unpacked engine behind.
pub struct TempDir {
    path: Option<PathBuf>,
}

impl TempDir {
    pub fn create(path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&path).with_context(|| format!("couldn't create {}", path.display()))?;
        Ok(Self { path: Some(path) })
    }

    pub fn path(&self) -> &Path {
        self.path.as_deref().expect("the folder is only taken by keep")
    }

    pub fn keep(mut self) -> PathBuf {
        self.path.take().expect("the folder is only taken by keep")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            _ = fs::remove_dir_all(path);
        }
    }
}

/// Whether two paths name the same place, ignoring case on Windows.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase() } else { a == b }
}

/// `path` made absolute without resolving links, so the launcher shows the path the user chose.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).with_context(|| format!("couldn't resolve {}", path.display()))
}
