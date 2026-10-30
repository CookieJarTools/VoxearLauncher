//! `voxup self update`: updates a standalone `voxup` from the launcher's own update manifest.

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use voxear_launcher::self_update::{self, UpdateChannel, VoxupUpdate};
use voxear_launcher::{Platform, launcher_version};

use crate::Launcher;
use crate::ui;

#[derive(Subcommand)]
pub enum SelfCommand {
    /// Update this voxup to the newest version in its channel.
    Update {
        /// stable or beta (default: the launcher's setting, or stable).
        #[arg(long)]
        channel: Option<UpdateChannel>,
        /// Only say whether there's an update.
        #[arg(long)]
        check: bool,
    },
}

pub fn run(launcher: &Launcher, command: SelfCommand) -> Result<()> {
    let SelfCommand::Update { channel, check } = command;
    let channel = channel.or(launcher.settings.launcher_channel).unwrap_or_default();
    let url = self_update::manifest_url(channel);
    let current = launcher_version();

    if check {
        match self_update::fetch_manifest(&url)? {
            None => println!("No launcher updates are published yet."),
            Some(manifest) if manifest.version > current => {
                ui::done(&format!("voxup {} is available ({channel}); you have {current}", manifest.version));
                ui::hint("Update with `voxup self update`.");
            }
            Some(_) => ui::done(&format!("voxup {current} is up to date ({channel})")),
        }
        return Ok(());
    }

    let key = self_update::update_key().ok_or_else(|| {
        anyhow!("this build of voxup has no launcher update key, so it can't update itself; build it again, or install a released one")
    })?;
    let exe = std::env::current_exe().context("couldn't find where voxup is")?;
    let platform = Platform::current().ok_or_else(|| anyhow!("Voxear isn't built for this platform"))?;
    let bar = ProgressBar::new(0);
    bar.set_style(
        ProgressStyle::with_template("Downloading  {bar:32.cyan/blue} {bytes:>10}/{total_bytes}")
            .expect("the template is valid")
            .progress_chars("━╸ "),
    );
    let outcome = self_update::update_voxup(&url, key, &current, &exe, platform, &mut |done, total| {
        bar.set_length(total.unwrap_or(done));
        bar.set_position(done);
    });
    bar.finish_and_clear();
    match outcome? {
        VoxupUpdate::NothingPublished => println!("No launcher updates are published yet."),
        VoxupUpdate::UpToDate(version) => ui::done(&format!("voxup {version} is up to date ({channel})")),
        VoxupUpdate::Updated { from, to } => ui::done(&format!("Updated voxup from {from} to {to}")),
    }
    Ok(())
}
