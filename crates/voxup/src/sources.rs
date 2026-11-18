//! `voxup releases`: where engine releases come from. The official releases are always listed, nightlies when
//! they are turned on, and custom releases, such as a fork's, beside them.

use anyhow::{Context, Result};
use clap::Subcommand;
use console::style;
use voxear_launcher::{Settings, sources};

use crate::Launcher;
use crate::ui;

#[derive(Subcommand)]
pub enum ReleasesCommand {
    /// List where engine releases come from. The default.
    List,
    /// List someone else's engine releases beside the official ones, such as a fork's.
    Add {
        /// Their releases.toml (an https:// URL or a file), the site or folder it is in, or the GitHub repository
        /// they publish from (owner/repo).
        address: String,
        /// What the engine list calls them (default: made from the address).
        #[arg(long)]
        name: Option<String>,
        /// The public key their releases are signed with: the base64 line, as in a minisign .pub file.
        #[arg(long)]
        key: Option<String>,
    },
    /// Stop listing custom releases. Engines installed from them stay installed.
    Remove { name: String },
    /// Show or hide nightly builds in the engine list.
    Nightlies {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
}

pub fn run(launcher: &Launcher, command: ReleasesCommand) -> Result<()> {
    match command {
        ReleasesCommand::List => list(launcher),
        ReleasesCommand::Add { address, name, key } => add(launcher, &address, name.as_deref(), key.as_deref()),
        ReleasesCommand::Remove { name } => {
            let mut settings = Settings::load(&launcher.layout)?;
            let removed = settings.remove_custom(&name)?;
            settings.save(&launcher.layout)?;
            ui::done(&format!("Removed {}; engines installed from them stay installed", removed.name));
            Ok(())
        }
        ReleasesCommand::Nightlies { state } => {
            let mut settings = Settings::load(&launcher.layout)?;
            settings.nightlies = Some(state == "on");
            settings.save(&launcher.layout)?;
            ui::done(if state == "on" { "Nightlies are listed" } else { "Nightlies aren't listed" });
            Ok(())
        }
    }
}

fn list(launcher: &Launcher) -> Result<()> {
    let installs = launcher.store.list()?;
    let nightlies = if launcher.settings.nightlies_shown(&installs) { "with nightlies" } else { "without nightlies" };
    let rows: Vec<Vec<String>> = launcher
        .sources()?
        .iter()
        .map(|source| {
            let note = match (source.official, launcher.settings.custom(&source.name)) {
                (true, _) => nightlies,
                (false, Some(custom)) if custom.key.is_some() => "own key",
                (false, _) => "",
            };
            vec![style(&source.name).bold().to_string(), pretty(&source.url), style(note).dim().to_string()]
        })
        .collect();
    ui::table(&rows);
    if launcher.settings.custom_releases.is_empty() {
        ui::hint("List a fork's releases beside these with `voxup releases add <address> --key <key>`.");
    }
    Ok(())
}

fn add(launcher: &Launcher, address: &str, name: Option<&str>, key: Option<&str>) -> Result<()> {
    let mut settings = Settings::load(&launcher.layout)?;
    let added = settings.add_custom(address, name, key)?.name.clone();
    let source = settings.sources()?.into_iter().find(|source| source.name == added).context("it was just added")?;
    // Read once before saving, so a wrong address or key shows now rather than in the engine list.
    let found = sources::check(&source, &launcher.layout).context("nothing was added; check the address and key")?;
    settings.save(&launcher.layout)?;
    match found {
        Some(count) => {
            let releases = if count == 1 { "1 release".to_owned() } else { format!("{count} releases") };
            ui::done(&format!("Added {added}, with {releases}, from {}", pretty(&source.url)));
        }
        None => {
            ui::done(&format!("Added {added}, from {}", pretty(&source.url)));
            ui::hint("Nothing is published there yet.");
        }
    }
    Ok(())
}

/// A URL as it is, a path with `~` for the home folder.
fn pretty(address: &str) -> String {
    if address.contains("://") { address.to_owned() } else { ui::pretty(std::path::Path::new(address)) }
}
