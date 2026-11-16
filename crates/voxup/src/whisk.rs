//! Whisk, the code editor Voxear opens scripts and projects in: install it, open it, remove it.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::Subcommand;
use voxear_launcher::whisk::{self, WhiskSource};

use crate::Launcher;
use crate::ui::{self, InstallBar};

#[derive(Subcommand)]
pub enum WhiskCommand {
    /// Install the newest Whisk. It keeps itself up to date after that.
    Install,
    /// Open Whisk, on a project folder if one is given.
    Open { folder: Option<PathBuf> },
    /// Remove the Whisk the launcher installed. A Whisk installed some other way is left alone.
    Remove,
    /// Say where Whisk is.
    Where,
}

pub fn run(launcher: &Launcher, command: WhiskCommand) -> Result<()> {
    let layout = &launcher.layout;
    match command {
        WhiskCommand::Install => {
            if let Some(found) = whisk::find(layout) {
                ui::done(&format!("Whisk is already installed at {}", ui::pretty(&found.path)));
                return Ok(());
            }
            let mut bar = InstallBar::new();
            let platform = launcher.store.platform();
            let installed = whisk::install(layout, &whisk::manifest_url(), whisk::WHISK_KEY, platform, &mut |step| {
                bar.update(step)
            })?;
            bar.finish();
            ui::done(&format!("Installed Whisk {} at {}", installed.version, ui::pretty(&installed.whisk.path)));
            Ok(())
        }
        WhiskCommand::Open { folder } => {
            let found =
                whisk::find(layout).ok_or_else(|| anyhow!("Whisk isn't installed; `voxup whisk install` adds it"))?;
            let folder = folder.map(|folder| std::path::absolute(&folder)).transpose()?;
            whisk::open(&found.path, folder.as_deref(), &[])
        }
        WhiskCommand::Remove => {
            if whisk::remove(layout)? {
                ui::done("Removed Whisk");
            } else if let Some(found) = whisk::find(layout) {
                ui::hint(&format!(
                    "Whisk at {} wasn't installed by the launcher, so it was left alone",
                    ui::pretty(&found.path)
                ));
            } else {
                println!("Whisk isn't installed.");
            }
            Ok(())
        }
        WhiskCommand::Where => {
            match whisk::find(layout) {
                Some(found) => {
                    let by = if found.source == WhiskSource::Launcher { " (installed by the launcher)" } else { "" };
                    println!("{}{by}", ui::pretty(&found.path));
                }
                None => println!("Whisk isn't installed; `voxup whisk install` adds it."),
            }
            Ok(())
        }
    }
}
