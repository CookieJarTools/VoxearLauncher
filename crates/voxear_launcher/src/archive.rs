//! Unpacks engine archives (`.tar.zst`). Archives are verified before they get here; even so, only plain
//! files and folders are accepted, with relative paths that stay inside the destination.

use std::cell::Cell;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path};
use std::rc::Rc;

use anyhow::{Context, Result, bail};

/// Unpacks `archive` into `dest`, calling `progress` with how many compressed bytes have been read so far.
pub fn unpack(archive: &Path, dest: &Path, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let file = File::open(archive).with_context(|| format!("couldn't open {}", archive.display()))?;
    unpack_reader(file, dest, progress)
}

/// Unpacks a module's archive into `dest`. Every entry must be inside the module's own folder, `<name>/`, so a
/// module can never change the engine's files.
pub fn unpack_module(archive: &Path, dest: &Path, name: &str, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let file = File::open(archive).with_context(|| format!("couldn't open {}", archive.display()))?;
    unpack_entries(file, dest, Some(name), progress)
}

pub fn unpack_reader(reader: impl Read, dest: &Path, progress: &mut dyn FnMut(u64)) -> Result<()> {
    unpack_entries(reader, dest, None, progress)
}

fn unpack_entries(reader: impl Read, dest: &Path, only_in: Option<&str>, progress: &mut dyn FnMut(u64)) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("couldn't create {}", dest.display()))?;
    let consumed = Rc::new(Cell::new(0));
    let counted = Counted { inner: reader, consumed: consumed.clone() };
    let decoder = zstd::Decoder::new(counted).context("the engine archive isn't zstd-compressed")?;
    let mut tar = tar::Archive::new(decoder);
    tar.set_overwrite(false);
    for entry in tar.entries().context("the engine archive isn't a tar archive")? {
        let mut entry = entry.context("the engine archive is damaged")?;
        let path = entry.path().context("the engine archive has an unreadable file name")?.into_owned();
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            bail!("the engine archive holds {} as a {kind:?}; only files and folders are allowed", path.display());
        }
        if let Some(folder) = only_in
            && first_folder(&path) != Some(folder)
        {
            bail!("the {folder} archive holds {}, which isn't in its {folder} folder", path.display());
        }
        if !stays_inside(&path)
            || !entry.unpack_in(dest).with_context(|| format!("couldn't unpack {}", path.display()))?
        {
            bail!("the engine archive holds {}, which points outside the install folder", path.display());
        }
        progress(consumed.get());
    }
    Ok(())
}

fn first_folder(path: &Path) -> Option<&str> {
    match path.components().find(|part| !matches!(part, Component::CurDir))? {
        Component::Normal(name) => name.to_str(),
        _ => None,
    }
}

fn stays_inside(path: &Path) -> bool {
    path.components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

struct Counted<R> {
    inner: R,
    consumed: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.consumed.set(self.consumed.get() + read as u64);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ArchiveEntry, tar_zst};

    #[test]
    fn unpacks_files_and_folders() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_zst(&[ArchiveEntry::dir("bin"), ArchiveEntry::file("bin/vox", b"#!/bin/sh\n", 0o755)]);
        let mut last = 0;
        unpack_reader(archive.as_slice(), dir.path(), &mut |done| last = done).unwrap();
        assert_eq!(std::fs::read(dir.path().join("bin/vox")).unwrap(), b"#!/bin/sh\n");
        assert!(last > 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("bin/vox")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "executables stay executable");
        }
    }

    #[test]
    fn refuses_links() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_zst(&[ArchiveEntry::symlink("evil", "/etc/passwd")]);
        let error = unpack_reader(archive.as_slice(), dir.path(), &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("only files and folders"), "{error}");
    }

    #[test]
    fn refuses_paths_that_leave_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        let archive = tar_zst(&[ArchiveEntry::file("../escaped", b"x", 0o644)]);
        assert!(unpack_reader(archive.as_slice(), &inner, &mut |_| {}).is_err());
        assert!(!dir.path().join("escaped").exists());
    }

    #[test]
    fn keeps_a_module_inside_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let manual = tar_zst(&[ArchiveEntry::dir("manual"), ArchiveEntry::file("manual/index.md", b"# Voxear", 0o644)]);
        let path = dir.path().join("manual.tar.zst");
        std::fs::write(&path, &manual).unwrap();
        unpack_module(&path, &dir.path().join("install"), "manual", &mut |_| {}).unwrap();
        assert!(dir.path().join("install/manual/index.md").is_file());

        let sneaky =
            tar_zst(&[ArchiveEntry::file("manual/index.md", b"x", 0o644), ArchiveEntry::file("bin/vox", b"x", 0o755)]);
        std::fs::write(&path, &sneaky).unwrap();
        let error = unpack_module(&path, &dir.path().join("other"), "manual", &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("isn't in its manual folder"), "{error}");
    }

    #[test]
    fn refuses_something_that_is_not_an_archive() {
        let dir = tempfile::tempdir().unwrap();
        assert!(unpack_reader(&b"not an archive"[..], dir.path(), &mut |_| {}).is_err());
    }
}
