//! Installing the launcher itself on Linux: the AppImage copied to `~/.local/bin/voxear-launcher`, a desktop
//! entry so it shows in the desktop's app menu, and its icons. Every desktop that follows the freedesktop.org
//! menu and icon specifications picks these up, on Wayland and X11 alike.
//!
//! Windows has a setup program of its own, so none of this runs there.
//!
//! Engine installs, the project list and settings live in the launcher folder (see [`crate::Layout`]), apart
//! from the program, so installing over an older copy, updating and uninstalling all leave them alone.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};

use crate::fsutil;

/// The name of the installed program, its desktop entry and its icon.
pub const APP_ID: &str = "voxear-launcher";
const NAME: &str = "Voxear Launcher";
const COMMENT: &str = "Install Voxear engine versions and open Voxear projects";

/// Where an install goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// `~/.local/bin`, which is on the PATH, so `voxear-launcher` also runs from a terminal.
    pub bin_dir: PathBuf,
    /// `~/.local/share`, or `$XDG_DATA_HOME`: desktop entries and icons go under it.
    pub data_dir: PathBuf,
}

impl Places {
    /// The current user's folders: `$XDG_BIN_HOME` or `~/.local/bin`, and `$XDG_DATA_HOME` or `~/.local/share`.
    pub fn from_env() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("couldn't find your home folder"))?;
        let bin_dir = dirs::executable_dir().unwrap_or_else(|| home.join(".local").join("bin"));
        let data_dir = dirs::data_dir().unwrap_or_else(|| home.join(".local").join("share"));
        Ok(Self { bin_dir, data_dir })
    }

    pub fn program(&self) -> PathBuf {
        self.bin_dir.join(APP_ID)
    }

    pub fn desktop_file(&self) -> PathBuf {
        self.data_dir.join("applications").join(format!("{APP_ID}.desktop"))
    }

    pub fn icon(&self, size: u32) -> PathBuf {
        self.data_dir
            .join("icons")
            .join("hicolor")
            .join(format!("{size}x{size}"))
            .join("apps")
            .join(format!("{APP_ID}.png"))
    }

    /// Whether `running` is the installed program.
    pub fn is_installed_copy(&self, running: &Path) -> bool {
        same_file(running, &self.program())
    }

    /// Whether the launcher is in the app menu: its program and its desktop entry are both there.
    pub fn is_installed(&self) -> bool {
        self.program().is_file() && self.desktop_file().is_file()
    }
}

/// A square PNG icon, `size` pixels across.
#[derive(Clone, Copy, Debug)]
pub struct Icon<'a> {
    pub size: u32,
    pub png: &'a [u8],
}

/// What [`install`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub program: PathBuf,
    pub desktop_file: PathBuf,
}

/// Installs the launcher at `source`, an AppImage, so it shows in the app menu. Installing over an older copy
/// replaces it in one rename, so whatever happens part way, the old copy or the new one is there, never half
/// of one. Installing the copy that is already installed only rewrites its desktop entry and icons.
pub fn install(source: &Path, places: &Places, icons: &[Icon]) -> Result<Installed> {
    ensure!(source.is_file(), "there's no launcher at {}", source.display());
    let program = places.program();
    if !same_file(source, &program) {
        copy_program(source, &program)?;
    }
    for icon in icons {
        fsutil::write_atomic(&places.icon(icon.size), icon.png)?;
    }
    let desktop_file = places.desktop_file();
    fsutil::write_atomic(&desktop_file, desktop_entry(&program).as_bytes())?;
    Ok(Installed { program, desktop_file })
}

/// Takes the launcher out of the app menu and removes the installed program and its icons. Returns what was
/// removed. Engine installs, projects and settings are kept.
pub fn uninstall(places: &Places, icon_sizes: &[u32]) -> Result<Vec<PathBuf>> {
    let mut paths = vec![places.desktop_file(), places.program()];
    paths.extend(icon_sizes.iter().map(|&size| places.icon(size)));
    let mut removed = Vec::new();
    for path in paths {
        match fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("couldn't remove {}", path.display())),
        }
    }
    Ok(removed)
}

/// The desktop entry for the program at `program`, as the freedesktop.org Desktop Entry specification lays it
/// out.
pub fn desktop_entry(program: &Path) -> String {
    let path = program.to_string_lossy();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.5\n\
         Name={NAME}\n\
         Comment={COMMENT}\n\
         Exec={exec}\n\
         TryExec={try_exec}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Development;IDE;\n\
         Keywords=Voxear;Engine;Game;Projects;\n\
         StartupNotify=true\n\
         StartupWMClass={APP_ID}\n",
        exec = exec_argument(&path),
        try_exec = escape_string(&path),
    )
}

/// One argument of an `Exec` key: quoted, with the characters the specification reserves inside quotes
/// escaped, `%` doubled, and the string escapes applied last.
fn exec_argument(argument: &str) -> String {
    let mut quoted = String::from("\"");
    for c in argument.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    escape_string(&quoted)
}

/// The escapes of a desktop entry's string values.
fn escape_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Copies `source` beside `program`, makes it executable, and renames it into place.
fn copy_program(source: &Path, program: &Path) -> Result<()> {
    let folder = program.parent().expect("the program is in a folder");
    fs::create_dir_all(folder).with_context(|| format!("couldn't create {}", folder.display()))?;
    let staged = fsutil::unique_sibling(program, "install");
    let result = (|| -> Result<()> {
        let mut from = File::open(source).with_context(|| format!("couldn't read {}", source.display()))?;
        let mut to = File::create(&staged).with_context(|| format!("couldn't write in {}", folder.display()))?;
        std::io::copy(&mut from, &mut to).with_context(|| format!("couldn't copy {}", source.display()))?;
        to.flush()?;
        to.sync_all()?;
        make_executable(&staged)?;
        fs::rename(&staged, program).with_context(|| format!("couldn't put the launcher at {}", program.display()))
    })();
    if result.is_err() {
        _ = fs::remove_file(&staged);
    }
    result
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("couldn't make {} executable", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Whether two paths are the same file, following links. Paths that don't exist are compared as written.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => fsutil::same_path(a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_32: &[u8] = b"\x89PNG 32";
    const PNG_128: &[u8] = b"\x89PNG 128";
    const ICONS: &[Icon] = &[Icon { size: 32, png: PNG_32 }, Icon { size: 128, png: PNG_128 }];

    struct Fixture {
        dir: tempfile::TempDir,
        places: Places,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let places = Places { bin_dir: home.join(".local/bin"), data_dir: home.join(".local/share") };
            Self { dir, places }
        }

        /// A downloaded AppImage, in a Downloads folder.
        fn download(&self, contents: &str) -> PathBuf {
            let path = self.dir.path().join("Downloads").join("Voxear Launcher_0.1.0_amd64.AppImage");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }
    }

    #[test]
    fn installs_into_the_app_menu() {
        let fixture = Fixture::new();
        let source = fixture.download("launcher 0.1.0");
        let installed = install(&source, &fixture.places, ICONS).unwrap();

        assert_eq!(installed.program, fixture.places.program());
        assert_eq!(fs::read_to_string(&installed.program).unwrap(), "launcher 0.1.0");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&installed.program).unwrap().permissions().mode() & 0o777, 0o755);
        }
        assert_eq!(fs::read(fixture.places.icon(32)).unwrap(), PNG_32);
        assert_eq!(fs::read(fixture.places.icon(128)).unwrap(), PNG_128);
        assert!(fixture.places.icon(128).ends_with("icons/hicolor/128x128/apps/voxear-launcher.png"));

        let entry = fs::read_to_string(&installed.desktop_file).unwrap();
        assert!(installed.desktop_file.ends_with("applications/voxear-launcher.desktop"));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\n"), "{entry}");
        assert!(entry.contains(&format!("Exec=\"{}\"\n", installed.program.display())), "{entry}");
        assert!(entry.contains("Icon=voxear-launcher\n"), "{entry}");
        assert!(fixture.places.is_installed());
        assert!(fixture.places.is_installed_copy(&installed.program));
        assert!(!fixture.places.is_installed_copy(&source));
    }

    #[test]
    fn installing_again_replaces_the_program_and_keeps_the_launcher_folder() {
        let fixture = Fixture::new();
        install(&fixture.download("launcher 0.1.0"), &fixture.places, ICONS).unwrap();
        // Engines, projects and settings, which live in the launcher folder beside the desktop entries.
        let settings = fixture.places.data_dir.join("voxear").join("settings.toml");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, "launcher_channel = \"beta\"\n").unwrap();

        install(&fixture.download("launcher 0.2.0"), &fixture.places, ICONS).unwrap();
        assert_eq!(fs::read_to_string(fixture.places.program()).unwrap(), "launcher 0.2.0");
        assert_eq!(fs::read_to_string(&settings).unwrap(), "launcher_channel = \"beta\"\n");
        let bin: Vec<_> = fs::read_dir(&fixture.places.bin_dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(bin, ["voxear-launcher"], "nothing staged is left behind");
    }

    #[test]
    fn installing_the_installed_copy_only_refreshes_the_menu_entry() {
        let fixture = Fixture::new();
        let installed = install(&fixture.download("launcher 0.1.0"), &fixture.places, ICONS).unwrap();
        fs::remove_file(&installed.desktop_file).unwrap();
        install(&installed.program, &fixture.places, ICONS).unwrap();
        assert_eq!(fs::read_to_string(&installed.program).unwrap(), "launcher 0.1.0");
        assert!(installed.desktop_file.is_file());
    }

    #[test]
    fn a_failed_install_leaves_the_old_copy_working() {
        let fixture = Fixture::new();
        install(&fixture.download("launcher 0.1.0"), &fixture.places, ICONS).unwrap();
        let missing = fixture.dir.path().join("Downloads").join("gone.AppImage");
        assert!(install(&missing, &fixture.places, ICONS).is_err());
        assert_eq!(fs::read_to_string(fixture.places.program()).unwrap(), "launcher 0.1.0");
    }

    #[test]
    fn uninstalls_the_program_and_menu_entry_but_keeps_the_launcher_folder() {
        let fixture = Fixture::new();
        install(&fixture.download("launcher 0.1.0"), &fixture.places, ICONS).unwrap();
        let engine = fixture.places.data_dir.join("voxear").join("engines").join("0.3.0").join("engine.toml");
        fs::create_dir_all(engine.parent().unwrap()).unwrap();
        fs::write(&engine, "version = \"0.3.0\"\n").unwrap();

        let removed = uninstall(&fixture.places, &[32, 128, 256]).unwrap();
        assert_eq!(removed.len(), 4, "{removed:?}");
        assert!(!fixture.places.program().exists());
        assert!(!fixture.places.desktop_file().exists());
        assert!(!fixture.places.icon(32).exists());
        assert!(!fixture.places.is_installed());
        assert!(engine.is_file());
        assert!(uninstall(&fixture.places, &[32]).unwrap().is_empty(), "uninstalling twice is fine");
    }

    #[test]
    fn quotes_the_program_path_in_the_menu_entry() {
        let entry = desktop_entry(Path::new("/home/a b/100%/$x\"y\\z/voxear-launcher"));
        let exec = entry.lines().find(|line| line.starts_with("Exec=")).unwrap();
        // Inside quotes: \" \$ and \\ escaped, % doubled; then every backslash escaped again as a string.
        assert_eq!(exec, r#"Exec="/home/a b/100%%/\\$x\\"y\\\\z/voxear-launcher""#);
        let try_exec = entry.lines().find(|line| line.starts_with("TryExec=")).unwrap();
        assert_eq!(try_exec, r#"TryExec=/home/a b/100%/$x"y\\z/voxear-launcher"#);
    }

    #[test]
    fn follows_links_when_telling_copies_apart() {
        let fixture = Fixture::new();
        let installed = install(&fixture.download("launcher 0.1.0"), &fixture.places, ICONS).unwrap();
        #[cfg(unix)]
        {
            let link = fixture.dir.path().join("link");
            std::os::unix::fs::symlink(&installed.program, &link).unwrap();
            assert!(fixture.places.is_installed_copy(&link));
        }
    }
}
