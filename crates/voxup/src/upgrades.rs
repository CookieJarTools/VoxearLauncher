//! Moving projects between engine versions, and their backups.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use clap::Args;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use semver::Version;
use voxear_launcher::project::{self, ProjectInfo};
use voxear_launcher::upgrade::{self, Direction, Event};
use voxear_launcher::{Selector, backup, releases};

use crate::Launcher;
use crate::projects::install_exact;
use crate::ui;

#[derive(Args)]
pub struct UpgradeArgs {
    /// The project folder.
    #[arg(default_value = ".")]
    folder: PathBuf,
    /// The engine version to move to: `stable`, `0.3` or `0.3.1`.
    #[arg(long, short, value_name = "VERSION", default_value = "stable")]
    to: String,
    /// Install the version first if it isn't installed.
    #[arg(long)]
    install: bool,
    /// Allow moving to an older version. Projects don't always open in older versions.
    #[arg(long)]
    allow_downgrade: bool,
}

pub fn upgrade(launcher: &Launcher, args: UpgradeArgs) -> Result<()> {
    let root = project::project_root(&args.folder);
    let info = ProjectInfo::read(&root)?;
    let selector: Selector = args.to.parse()?;
    let target = match launcher.store.pick(&selector)? {
        Some(install) => install,
        None if args.install => install_exact(launcher, &args.to)?,
        None => bail!("no installed engine version matches `{}`; install it, or pass --install", args.to),
    };
    let verb = match Direction::of(info.engine.as_ref(), &target.version) {
        Some(Direction::Pin) => "Pinning",
        Some(Direction::Upgrade) => "Upgrading",
        Some(Direction::Downgrade) => "Downgrading",
        None => bail!("{} is already on Voxear {}", info.name, target.version),
    };
    println!("{verb} {} to Voxear {}", style(&info.name).bold(), target.version);

    let bar = ProgressBar::new(0);
    bar.set_style(
        ProgressStyle::with_template("{msg:12} {bar:32.cyan/blue} {bytes:>10}/{total_bytes}")
            .expect("the template is valid")
            .progress_chars("━╸ "),
    );
    bar.set_message("Backing up");
    let report = upgrade::upgrade(&launcher.layout, &root, &target, args.allow_downgrade, &mut |event| match event {
        Event::BackingUp { done, total } => {
            bar.set_length(total);
            bar.set_position(done);
            if done == total {
                bar.finish_and_clear();
            }
        }
        Event::Step(step) => bar.suspend(|| ui::hint(&format!("• {step}"))),
        Event::Change(_) => {}
    });
    bar.finish_and_clear();
    let report = report?;

    ui::done(&format!("{} is on Voxear {}", info.name, report.to));
    for change in &report.changes {
        println!("  {} {change}", style("✓").green());
    }
    if !report.migrated {
        ui::hint(&format!(
            "Voxear {} has no `vox upgrade` yet, so only the pinned version changed; nothing else in the project was converted.",
            report.to
        ));
    }
    ui::hint(&format!("Backup: {}", ui::pretty(&report.backup.archive)));
    for (version, notes) in notes_between(launcher, report.from.as_ref(), &report.to) {
        ui::hint(&format!("What's new in {version}: {notes}"));
    }
    Ok(())
}

/// Release notes for the versions passed through, when the release list can be read. Quietly empty if not.
fn notes_between(launcher: &Launcher, from: Option<&Version>, to: &Version) -> Vec<(Version, String)> {
    let Some(url) = launcher.settings.releases_url() else { return Vec::new() };
    let Ok(keys) = launcher.keys() else { return Vec::new() };
    let Ok(loaded) = releases::load(&url, &keys, &launcher.layout) else { return Vec::new() };
    loaded
        .manifest
        .between(from, to)
        .into_iter()
        .filter_map(|release| release.notes.clone().map(|notes| (release.version.clone(), notes)))
        .collect()
}

pub fn backups(launcher: &Launcher, folder: Option<&Path>) -> Result<()> {
    let folder = folder.map(project::project_root);
    let backups = backup::list(&launcher.layout, folder.as_deref())?;
    if backups.is_empty() {
        println!("No backups yet.");
        ui::hint("One is made every time a project changes engine version.");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = backups
        .iter()
        .map(|backup| {
            let from = backup.from.as_ref().map_or_else(|| "unpinned".to_owned(), ToString::to_string);
            vec![
                style(&backup.project).bold().to_string(),
                format!("{from} → {}", backup.to),
                backup.created.strftime("%Y-%m-%d %H:%M").to_string(),
                human_size(backup.size),
                ui::pretty(&backup.archive),
            ]
        })
        .collect();
    ui::table(&rows);
    Ok(())
}

pub fn restore(launcher: &Launcher, archive: &Path) -> Result<()> {
    let backup = backup::find(&launcher.layout, archive)?;
    let restored = backup::restore(&backup)?;
    let mut registry = launcher.registry()?;
    registry.add(&restored).map_err(|e| anyhow!("restored to {}, but couldn't list it: {e:#}", restored.display()))?;
    registry.save()?;
    ui::done(&format!("Restored {} to {}", backup.project, ui::pretty(&restored)));
    ui::hint("It's in the project list too. The project it came from wasn't touched.");
    Ok(())
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
