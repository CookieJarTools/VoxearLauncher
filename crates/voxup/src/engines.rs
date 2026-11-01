//! Engine versions: list, install, uninstall, locate.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use console::style;
use semver::Version;
use voxear_launcher::manifest::{MANUAL, SIGNATURE_SUFFIX};
use voxear_launcher::{Install, InstallKind, Removed, Selector};

use crate::Launcher;
use crate::ui::{self, InstallBar};

pub fn list(launcher: &Launcher, available: bool) -> Result<()> {
    let installs = launcher.store.list()?;
    let mut rows: Vec<Vec<String>> = installs.iter().map(install_row).collect();

    if available {
        let releases = match launcher.releases() {
            Ok(releases) => releases,
            Err(error) if voxear_launcher::fetch::is_not_found(&error) => {
                if rows.is_empty() {
                    println!("No engine versions are installed, and none are published yet.");
                } else {
                    ui::table(&rows);
                    ui::hint("No engine releases are published yet.");
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        for release in releases.manifest.available(launcher.store.platform()) {
            if installs.iter().any(|install| install.version == release.version) {
                continue;
            }
            let note = if release.needs_newer_launcher() { "needs a newer launcher" } else { "available" };
            let row = vec![
                release.version.to_string(),
                release.channel.to_string(),
                String::new(),
                style(note).dim().to_string(),
            ];
            rows.push(row);
        }
        rows.sort_by_key(|row| std::cmp::Reverse(version_of(row)));
    }

    if rows.is_empty() {
        println!("No engine versions are installed yet.");
        ui::hint("Install one with `voxup install stable`.");
        return Ok(());
    }
    ui::table(&rows);
    Ok(())
}

fn install_row(install: &Install) -> Vec<String> {
    let mut path = ui::pretty(&install.path);
    if install.kind == InstallKind::Located {
        path.push_str(&style(" (located)").dim().to_string());
    }
    if install.missing {
        path.push_str(&style(" (folder not found)").yellow().to_string());
    }
    let manual = if install.has_module(MANUAL) { style("manual").dim().to_string() } else { String::new() };
    vec![style(&install.version).bold().to_string(), install.channel.to_string(), manual, path]
}

fn version_of(row: &[String]) -> Option<Version> {
    Version::parse(&console::strip_ansi_codes(&row[0])).ok()
}

pub fn install(
    launcher: &Launcher,
    version: Option<String>,
    file: Option<PathBuf>,
    without_manual: bool,
) -> Result<()> {
    let keys = launcher.keys()?;
    let mut bar = InstallBar::new();

    if let Some(file) = file {
        let signature_path = signature_path(&file);
        let signature = std::fs::read_to_string(&signature_path)
            .with_context(|| format!("there's no signature next to it (expected {})", signature_path.display()))?;
        let install = launcher.store.install_archive(&file, &signature, &keys, &mut |p| bar.update(p))?;
        bar.finish();
        ui::done(&format!("Installed Voxear {} ({}) from {}", install.version, install.channel, ui::pretty(&file)));
        return Ok(());
    }

    let selector: Selector = version.expect("clap requires a version or --file").parse()?;
    let releases = launcher.releases()?;
    let platform = launcher.store.platform();
    let release = releases
        .manifest
        .select(&selector, platform)
        .ok_or_else(|| anyhow!("no release matches {selector} for {platform}"))?;
    if let Some(install) = launcher.store.get(&release.version)? {
        ui::done(&format!("Voxear {} is already installed, at {}", install.version, ui::pretty(&install.path)));
        return Ok(());
    }

    let without = if without_manual { vec![MANUAL.to_owned()] } else { Vec::new() };
    let cancel = AtomicBool::new(false);
    let install =
        launcher.store.install_without(release, &releases.url, &keys, &without, &mut |p| bar.update(p), &cancel)?;
    bar.finish();
    let with = if install.has_module(MANUAL) { ", with the manual" } else { "" };
    ui::done(&format!(
        "Installed Voxear {} ({}{with}) in {}",
        install.version,
        install.channel,
        ui::pretty(&install.path)
    ));
    if without_manual && release.module(MANUAL).is_some() {
        ui::hint(&format!("Add the manual later with `voxup manual add {}`.", install.version));
    }
    Ok(())
}

#[derive(clap::Subcommand)]
pub enum ManualCommand {
    /// Download and add the Voxear Manual to an installed version.
    Add { version: String },
    /// Remove the Voxear Manual from an installed version, keeping the engine.
    Remove { version: String },
}

pub fn manual(launcher: &Launcher, command: ManualCommand) -> Result<()> {
    match command {
        ManualCommand::Add { version } => {
            let version = exact(&version)?;
            let releases = launcher.releases()?;
            let release = releases
                .manifest
                .find(&version)
                .ok_or_else(|| anyhow!("Voxear {version} isn't in the release list, so its manual can't be found"))?;
            let keys = launcher.keys()?;
            let mut bar = InstallBar::new();
            let cancel = AtomicBool::new(false);
            launcher.store.add_module(release, MANUAL, &releases.url, &keys, &mut |p| bar.update(p), &cancel)?;
            bar.finish();
            ui::done(&format!("Added the manual to Voxear {version}"));
        }
        ManualCommand::Remove { version } => {
            let version = exact(&version)?;
            launcher.store.remove_module(&version, MANUAL)?;
            ui::done(&format!("Removed the manual from Voxear {version}"));
        }
    }
    Ok(())
}

fn exact(version: &str) -> Result<Version> {
    Version::parse(version).with_context(|| format!("`{version}` isn't an exact version, such as 0.3.0"))
}

fn signature_path(archive: &Path) -> PathBuf {
    let mut name = archive.as_os_str().to_owned();
    name.push(SIGNATURE_SUFFIX);
    PathBuf::from(name)
}

pub fn uninstall(launcher: &Launcher, version: &str, force: bool) -> Result<()> {
    let version =
        Version::parse(version).with_context(|| format!("`{version}` isn't an exact version, such as 0.3.0"))?;
    let users = launcher.registry()?.using(&version);
    if !users.is_empty() && !force {
        let names: Vec<_> = users.iter().map(|project| project.entry.name.as_str()).collect();
        bail!(
            "{} {} Voxear {version}: {}. Pass --force to uninstall it anyway",
            users.len(),
            if users.len() == 1 { "project uses" } else { "projects use" },
            names.join(", ")
        );
    }
    match launcher.store.remove(&version)? {
        Removed::Deleted => ui::done(&format!("Uninstalled Voxear {version}")),
        Removed::Forgotten(path) => {
            ui::done(&format!("Forgot Voxear {version}; its files in {} were left alone", ui::pretty(&path)))
        }
    }
    Ok(())
}

pub fn locate(launcher: &Launcher, folder: &Path) -> Result<()> {
    let install = launcher.store.locate(folder)?;
    ui::done(&format!("Added Voxear {} ({}) at {}", install.version, install.channel, ui::pretty(&install.path)));
    Ok(())
}
