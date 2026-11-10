//! The launcher's few flags, for terminals and scripts. Everything else is `voxup`'s job.

use std::ffi::OsString;

use anyhow::{Result, bail};
use tauri::{Context, Wry};
use voxear_launcher::launcher_version;
use voxear_launcher::self_install::{self, Places};

use crate::commands::Launcher;
use crate::updates::{self, Lookup};

const HELP: &str = "\
Voxear Launcher

Usage: voxear-launcher [option]

With no option, the launcher window opens.

Options:
  --update      Update to the newest launcher in your channel, without opening the window
  --install     Put this AppImage in the app menu: ~/.local/bin/voxear-launcher (Linux)
  --uninstall   Take the launcher out of the app menu and remove it; engines, projects and settings stay (Linux)
  --version     Print the launcher's version
  --help        Print this
";

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Window,
    Update,
    Install,
    Uninstall,
    Version,
    Help,
}

/// Reads the arguments after the program's name. Anything that isn't a flag, such as a path the desktop passes
/// along, opens the window.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action> {
    let flags: Vec<String> =
        args.into_iter().map(|arg| arg.to_string_lossy().into_owned()).filter(|arg| arg.starts_with('-')).collect();
    let action = match flags.as_slice() {
        [] => Action::Window,
        [flag] => match flag.as_str() {
            "--update" => Action::Update,
            "--install" => Action::Install,
            "--uninstall" => Action::Uninstall,
            "--version" | "-V" => Action::Version,
            "--help" | "-h" => Action::Help,
            other => bail!("`{other}` isn't an option of the launcher; see --help"),
        },
        _ => bail!("the launcher takes one option at a time; see --help"),
    };
    Ok(action)
}

/// Runs a flag that needs no window, and returns the exit code.
pub fn run(action: Action, context: Context<Wry>) -> i32 {
    attach_console();
    let outcome = match action {
        Action::Window => unreachable!("the window is opened by the caller"),
        Action::Version => {
            println!("Voxear Launcher {}", launcher_version());
            Ok(())
        }
        Action::Help => {
            print!("{HELP}");
            Ok(())
        }
        Action::Install => install(),
        Action::Uninstall => uninstall(),
        Action::Update => return update(context),
    };
    match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    }
}

fn install() -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("on Windows, the setup program installs the launcher");
    }
    let source = match updates::appimage() {
        Some(appimage) => appimage,
        None => std::env::current_exe()?,
    };
    let places = Places::from_env()?;
    let installed = self_install::install(&source, &places, updates::ICONS)?;
    println!("Installed Voxear Launcher {}", launcher_version());
    println!("  Program   {}", installed.program.display());
    println!("  App menu  {}", installed.desktop_file.display());
    if updates::appimage().is_none() {
        println!("This copy isn't an AppImage, so it won't update itself.");
    }
    Ok(())
}

fn uninstall() -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("on Windows, uninstall the launcher from Settings › Apps");
    }
    let places = Places::from_env()?;
    let removed = self_install::uninstall(&places, &updates::icon_sizes())?;
    if removed.is_empty() {
        println!("The launcher isn't installed in {}", places.bin_dir.display());
        return Ok(());
    }
    println!("Uninstalled Voxear Launcher");
    for path in &removed {
        println!("  Removed  {}", path.display());
    }
    let layout = voxear_launcher::Layout::from_env()?;
    println!("  Kept     {} (engines, projects and settings)", layout.root().display());
    Ok(())
}

/// Updates without a window: Tauri's updater needs a running app, so one starts with no windows, updates, and
/// exits.
fn update(mut context: Context<Wry>) -> i32 {
    if let Some(reason) = updates::why_no_self_update() {
        eprintln!("error: {reason}");
        return 1;
    }
    context.config_mut().app.windows.clear();
    let built = tauri::Builder::default().plugin(crate::updater()).setup(|app| {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let outcome = update_now(&handle).await;
            if let Err(error) = &outcome {
                eprintln!("error: {error:#}");
            }
            handle.exit(if outcome.is_ok() { 0 } else { 1 });
        });
        Ok(())
    });
    match built.build(context) {
        Ok(app) => app.run_return(|_, _| {}),
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

async fn update_now(app: &tauri::AppHandle) -> Result<()> {
    let launcher = Launcher::new()?;
    let channel = launcher.channel()?;
    let current = launcher_version();
    let found = match updates::look_for_update(app, channel).await? {
        Lookup::NothingPublished => {
            println!("No launcher updates are published in the {channel} channel yet.");
            return Ok(());
        }
        Lookup::UpToDate(version) => {
            println!("Voxear Launcher {version} is up to date ({channel})");
            return Ok(());
        }
        Lookup::Available(found) => found,
    };
    let version = found.update.version.clone();
    println!("Updating Voxear Launcher {current} to {version}…");
    let mut shown = 0u64;
    // On Windows the setup program takes over and the launcher exits; it shouldn't open the window afterwards.
    let found = updates::Found { update: found.update.restart_after_install(false), entry: found.entry };
    updates::install(found, |done, total| {
        // A line every 10 MB, so a log stays readable.
        if done >= shown + 10_000_000 || Some(done) == total {
            shown = done;
            match total {
                Some(total) => println!("  {} of {} MB", done / 1_000_000, total / 1_000_000),
                None => println!("  {} MB", done / 1_000_000),
            }
        }
    })
    .await?;
    println!("Updated to {version}. It starts the next time you open the launcher.");
    Ok(())
}

/// A release build on Windows has no console of its own, so its output goes to the terminal it was started
/// from, if there is one.
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: AttachConsole takes a process ID and has no other preconditions; failing is harmless.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Action> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn reads_the_flags() {
        assert_eq!(parse_args(&[]).unwrap(), Action::Window);
        assert_eq!(parse_args(&["--update"]).unwrap(), Action::Update);
        assert_eq!(parse_args(&["--install"]).unwrap(), Action::Install);
        assert_eq!(parse_args(&["--uninstall"]).unwrap(), Action::Uninstall);
        assert_eq!(parse_args(&["-V"]).unwrap(), Action::Version);
        assert_eq!(parse_args(&["--help"]).unwrap(), Action::Help);
        assert_eq!(parse_args(&["/home/a/Orbit"]).unwrap(), Action::Window, "a path opens the window");
        assert!(parse_args(&["--frobnicate"]).is_err());
        assert!(parse_args(&["--install", "--update"]).is_err());
    }

    #[test]
    fn help_lists_every_flag() {
        for flag in ["--update", "--install", "--uninstall", "--version", "--help"] {
            assert!(HELP.contains(flag), "{flag}");
        }
    }
}
