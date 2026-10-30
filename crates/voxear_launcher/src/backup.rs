//! Project backups, made before a project changes engine version.
//!
//! A backup is a `.tar.zst` of the project folder in `backups/<project>/`, with a `.toml` beside it saying what
//! it is. It leaves out what can be rebuilt: `.voxear/` (caches), `bin/` and `obj/` next to a C# project, and the
//! IDE's `.vs/`. It also leaves out `.git/`, which is its own history. Restoring never overwrites anything: the
//! backup is unpacked into a new folder beside the project.

use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive;
use crate::fsutil::{self, TempDir};
use crate::layout::Layout;
use crate::store::hex;

/// Left out at the project's root.
const SKIP_AT_ROOT: &[&str] = &[".voxear", ".git", ".vs"];
/// Left out next to a `.csproj`: the C# build's output.
const SKIP_BESIDE_CSPROJ: &[&str] = &["bin", "obj"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    /// The project's name when it was backed up.
    pub project: String,
    /// Where the project was.
    pub path: PathBuf,
    /// The version it was pinned to; none if it wasn't pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Version>,
    /// The version it was moving to.
    pub to: Version,
    pub created: Timestamp,
    /// The size of the archive, in bytes.
    pub size: u64,
    pub files: u64,
    /// Links, which backups don't hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// What the upgrade changed, filled in once it finished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    /// The archive. Not stored: it is wherever the description sits.
    #[serde(skip)]
    pub archive: PathBuf,
}

impl Backup {
    fn description(&self) -> PathBuf {
        self.archive.with_extension("").with_extension("toml")
    }

    /// Saves the description beside the archive.
    pub fn save(&self) -> Result<()> {
        fsutil::write_atomic(&self.description(), toml::to_string(self)?.as_bytes())
    }
}

/// Backs up the project at `root`, calling `progress` with bytes done and the total.
pub fn create(
    layout: &Layout,
    root: &Path,
    project: &str,
    from: Option<&Version>,
    to: &Version,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Backup> {
    let root = fsutil::absolute(root)?;
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    collect(&root, Path::new(""), true, &mut entries, &mut skipped)?;
    let total: u64 = entries.iter().map(|entry| entry.size).sum();

    let folder = layout.backups().join(project_folder(project, &root));
    fs::create_dir_all(&folder).with_context(|| format!("couldn't create {}", folder.display()))?;
    let created = Timestamp::now();
    let archive = unique_archive(&folder, &created);
    let part = fsutil::unique_sibling(&archive, "part");

    let result = write_archive(&root, &entries, &part, total, progress);
    if let Err(error) = result {
        _ = fs::remove_file(&part);
        return Err(error.context(format!("couldn't back up {project}")));
    }
    fs::rename(&part, &archive).with_context(|| format!("couldn't save the backup at {}", archive.display()))?;

    let backup = Backup {
        project: project.to_owned(),
        path: root,
        from: from.cloned(),
        to: to.clone(),
        created,
        size: fs::metadata(&archive).map(|meta| meta.len()).unwrap_or(0),
        files: entries.iter().filter(|entry| !entry.dir).count() as u64,
        skipped,
        changes: Vec::new(),
        archive,
    };
    backup.save()?;
    Ok(backup)
}

/// Every backup, or only those of the project at `project`, newest first.
pub fn list(layout: &Layout, project: Option<&Path>) -> Result<Vec<Backup>> {
    let project = project.map(fsutil::absolute).transpose()?;
    let mut backups = Vec::new();
    let folders = match fs::read_dir(layout.backups()) {
        Ok(folders) => folders,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(backups),
        Err(e) => return Err(e).context("couldn't read the backups folder"),
    };
    for folder in folders.flatten() {
        let Ok(files) = fs::read_dir(folder.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let Some(backup) = read(&path) else { continue };
            if project.as_ref().is_none_or(|project| fsutil::same_path(project, &backup.path)) {
                backups.push(backup);
            }
        }
    }
    backups.sort_by_key(|backup| std::cmp::Reverse(backup.created));
    Ok(backups)
}

/// The backup whose archive is at `archive`.
pub fn find(layout: &Layout, archive: &Path) -> Result<Backup> {
    let archive = fsutil::absolute(archive)?;
    list(layout, None)?
        .into_iter()
        .find(|backup| fsutil::same_path(&backup.archive, &archive))
        .ok_or_else(|| anyhow!("{} isn't one of the launcher's backups", archive.display()))
}

/// Unpacks a backup into a new folder beside the project, such as `Orbit (before 0.3.0)`, and returns it.
/// Nothing that exists is ever overwritten.
pub fn restore(backup: &Backup) -> Result<PathBuf> {
    let parent = backup.path.parent().ok_or_else(|| anyhow!("{} has no parent folder", backup.path.display()))?;
    if !parent.is_dir() {
        bail!("{} isn't there any more, so there's nowhere to restore to", parent.display());
    }
    let name = backup.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = free_folder(parent, &format!("{name} (before {})", backup.to));
    let staging = TempDir::create(fsutil::unique_sibling(&dest, "restoring"))?;
    archive::unpack(&backup.archive, staging.path(), &mut |_| {})?;
    fs::rename(staging.path(), &dest).with_context(|| format!("couldn't restore into {}", dest.display()))?;
    staging.keep();
    Ok(dest)
}

pub fn delete(backup: &Backup) -> Result<()> {
    fs::remove_file(&backup.archive).with_context(|| format!("couldn't delete {}", backup.archive.display()))?;
    _ = fs::remove_file(backup.description());
    if let Some(folder) = backup.archive.parent() {
        _ = fs::remove_dir(folder);
    }
    Ok(())
}

fn read(description: &Path) -> Option<Backup> {
    let mut backup: Backup = toml::from_str(&fs::read_to_string(description).ok()?).ok()?;
    backup.archive = description.with_extension("tar.zst");
    backup.archive.is_file().then_some(backup)
}

struct Entry {
    relative: PathBuf,
    dir: bool,
    size: u64,
}

fn collect(
    root: &Path,
    relative: &Path,
    at_root: bool,
    entries: &mut Vec<Entry>,
    skipped: &mut Vec<String>,
) -> Result<()> {
    let dir = root.join(relative);
    let mut children: Vec<_> = fs::read_dir(&dir)
        .with_context(|| format!("couldn't read {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("couldn't read {}", dir.display()))?;
    children.sort_by_key(|child| child.file_name());
    let has_csproj = children.iter().any(|child| child.path().extension().is_some_and(|ext| ext == "csproj"));

    for child in children {
        let name = child.file_name();
        let name_text = name.to_string_lossy();
        let path = relative.join(&name);
        let kind = child.file_type().with_context(|| format!("couldn't read {}", path.display()))?;
        if kind.is_symlink() {
            skipped.push(path.to_string_lossy().into_owned());
        } else if kind.is_dir() {
            let rebuildable = (at_root && SKIP_AT_ROOT.contains(&name_text.as_ref()))
                || (has_csproj && SKIP_BESIDE_CSPROJ.contains(&name_text.as_ref()));
            if !rebuildable {
                entries.push(Entry { relative: path.clone(), dir: true, size: 0 });
                collect(root, &path, false, entries, skipped)?;
            }
        } else if kind.is_file() {
            let size = child.metadata().map(|meta| meta.len()).unwrap_or(0);
            entries.push(Entry { relative: path, dir: false, size });
        }
    }
    Ok(())
}

fn write_archive(
    root: &Path,
    entries: &[Entry],
    part: &Path,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let file = File::create(part).with_context(|| format!("couldn't write {}", part.display()))?;
    let encoder = zstd::Encoder::new(file, 3)?;
    let mut tar = tar::Builder::new(encoder);
    let mut done = 0u64;
    progress(done, total);
    for entry in entries {
        let source = root.join(&entry.relative);
        if entry.dir {
            tar.append_dir(&entry.relative, &source)?;
            continue;
        }
        let mut file = File::open(&source).with_context(|| format!("couldn't read {}", source.display()))?;
        let mut header = tar::Header::new_gnu();
        header.set_metadata(&file.metadata()?);
        let mut counted = Counted { inner: &mut file, done: &mut done, total, progress };
        tar.append_data(&mut header, &entry.relative, &mut counted)
            .with_context(|| format!("couldn't back up {}", source.display()))?;
    }
    let encoder = tar.into_inner()?;
    encoder.finish()?.sync_all()?;
    progress(total, total);
    Ok(())
}

/// Reports bytes as a file is read into the archive.
struct Counted<'a, R> {
    inner: R,
    done: &'a mut u64,
    total: u64,
    progress: &'a mut dyn FnMut(u64, u64),
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        *self.done += read as u64;
        (self.progress)(*self.done, self.total);
        Ok(read)
    }
}

/// `Orbit-1a2b3c4d`: the name, plus a hash of the path, so two projects with one name stay apart.
fn project_folder(project: &str, root: &Path) -> String {
    let name: String =
        project.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let hash = hex(&Sha256::digest(root.to_string_lossy().as_bytes()));
    format!("{}-{}", name.trim_matches('-'), &hash[..8])
}

fn unique_archive(folder: &Path, created: &Timestamp) -> PathBuf {
    let stamp = created.strftime("%Y%m%dT%H%M%SZ").to_string();
    let mut candidate = folder.join(format!("{stamp}.tar.zst"));
    let mut count = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stamp}-{count}.tar.zst"));
        count += 1;
    }
    candidate
}

fn free_folder(parent: &Path, name: &str) -> PathBuf {
    let mut candidate = parent.join(name);
    let mut count = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{name} {count}"));
        count += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        layout: Layout,
        project: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let layout = Layout::new(dir.path().join("home"));
            let project = dir.path().join("games").join("Orbit");
            let write = |path: &str, text: &str| {
                let path = project.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            };
            write("voxear.toml", "[project]\nname = \"Orbit\"\nengine = \"0.2.0\"\n");
            write("Scenes/Main.scene", "scene");
            write("Scripts/Orbit.Scripts.csproj", "<Project />");
            write("Scripts/Player.cs", "class Player {}");
            write("Scripts/bin/Debug/Orbit.Scripts.dll", "built");
            write("Scripts/obj/project.assets.json", "{}");
            write(".voxear/cache/shaders.bin", "cache");
            write(".git/HEAD", "ref: refs/heads/master");
            write("Assets/bin/keep-me.txt", "a folder named bin that isn't build output");
            Self { _dir: dir, layout, project }
        }

        fn backup(&self) -> Backup {
            create(
                &self.layout,
                &self.project,
                "Orbit",
                Some(&Version::new(0, 2, 0)),
                &Version::new(0, 3, 0),
                &mut |_, _| {},
            )
            .unwrap()
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(folder) = stack.pop() {
            for entry in fs::read_dir(folder).unwrap().flatten() {
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                } else {
                    files.push(entry.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn backs_up_the_project_without_what_can_be_rebuilt() {
        let fixture = Fixture::new();
        let mut last = (0, 0);
        let backup =
            create(&fixture.layout, &fixture.project, "Orbit", None, &Version::new(0, 3, 0), &mut |done, total| {
                last = (done, total)
            })
            .unwrap();
        assert!(backup.archive.is_file());
        assert_eq!(backup.files, 5);
        assert_eq!(last.0, last.1);
        assert!(last.1 > 0);

        let restored = restore(&backup).unwrap();
        assert_eq!(
            files_in(&restored),
            [
                "Assets/bin/keep-me.txt",
                "Scenes/Main.scene",
                "Scripts/Orbit.Scripts.csproj",
                "Scripts/Player.cs",
                "voxear.toml"
            ]
        );
    }

    #[test]
    fn lists_backups_newest_first_and_per_project() {
        let fixture = Fixture::new();
        let first = fixture.backup();
        let second = fixture.backup();
        assert_ne!(first.archive, second.archive, "two backups in one second stay apart");

        let all = list(&fixture.layout, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].archive, second.archive);
        assert_eq!(all[0].from, Some(Version::new(0, 2, 0)));
        assert_eq!(list(&fixture.layout, Some(&fixture.project)).unwrap().len(), 2);
        assert!(list(&fixture.layout, Some(Path::new("/elsewhere"))).unwrap().is_empty());
        assert_eq!(find(&fixture.layout, &first.archive).unwrap().created, first.created);
    }

    #[test]
    fn restores_beside_the_project_without_overwriting_anything() {
        let fixture = Fixture::new();
        let backup = fixture.backup();
        let first = restore(&backup).unwrap();
        let second = restore(&backup).unwrap();
        assert_eq!(first.file_name().unwrap(), "Orbit (before 0.3.0)");
        assert_eq!(second.file_name().unwrap(), "Orbit (before 0.3.0) 2");
        assert!(fixture.project.join(".git/HEAD").is_file(), "the project itself is untouched");
    }

    #[test]
    fn keeps_what_the_upgrade_changed() {
        let fixture = Fixture::new();
        let mut backup = fixture.backup();
        backup.changes = vec!["Moved scenes to the new format".to_owned()];
        backup.save().unwrap();
        assert_eq!(list(&fixture.layout, None).unwrap()[0].changes, backup.changes);
    }

    #[test]
    fn deletes_a_backup() {
        let fixture = Fixture::new();
        let backup = fixture.backup();
        delete(&backup).unwrap();
        assert!(list(&fixture.layout, None).unwrap().is_empty());
        assert!(!backup.archive.exists());
    }

    #[cfg(unix)]
    #[test]
    fn leaves_out_links_and_says_so() {
        let fixture = Fixture::new();
        std::os::unix::fs::symlink("/etc", fixture.project.join("etc-link")).unwrap();
        let backup = fixture.backup();
        assert_eq!(backup.skipped, ["etc-link"]);
        restore(&backup).unwrap();
    }
}
