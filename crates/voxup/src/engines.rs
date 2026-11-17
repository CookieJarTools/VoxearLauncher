//! Engine versions: list, install, uninstall, locate.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use console::style;
use semver::Version;
use voxear_launcher::manifest::SIGNATURE_SUFFIX;
use voxear_launcher::sources::Ambiguous;
use voxear_launcher::{Channel, Install, InstallKind, Module, Removed, Selector};

use crate::Launcher;
use crate::ui::{self, InstallBar};

pub fn list(launcher: &Launcher, available: bool) -> Result<()> {
    let installs = launcher.store.list()?;
    let mut rows: Vec<Vec<String>> = installs.iter().map(|install| install_row(launcher, install)).collect();
    let mut hidden_nightlies = 0;

    if available {
        let catalog = match launcher.catalog() {
            Ok(catalog) => catalog,
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
        let nightlies = launcher.settings.nightlies_shown(&installs);
        for listed in catalog.available(launcher.store.platform()) {
            let release = listed.release;
            if installs.iter().any(|install| install.version == release.version) {
                continue;
            }
            if release.channel == Channel::Nightly && !nightlies {
                hidden_nightlies += 1;
                continue;
            }
            let note = if release.needs_newer_launcher() { "needs a newer launcher" } else { "available" };
            let from = if listed.source.official { String::new() } else { format!("from {}", listed.source.name) };
            let row = vec![
                release.version.to_string(),
                release.channel.to_string(),
                style(from).dim().to_string(),
                style(note).dim().to_string(),
            ];
            rows.push(row);
        }
        rows.sort_by_key(|row| std::cmp::Reverse(version_of(row)));
    }

    if rows.is_empty() {
        println!("No engine versions are installed yet.");
        ui::hint("Install one with `voxup install stable`.");
    } else {
        ui::table(&rows);
    }
    if hidden_nightlies > 0 {
        let count = if hidden_nightlies == 1 {
            "1 nightly isn't".to_owned()
        } else {
            format!("{hidden_nightlies} nightlies aren't")
        };
        ui::hint(&format!("{count} listed; show nightlies with `voxup releases nightlies on`."));
    }
    Ok(())
}

fn install_row(launcher: &Launcher, install: &Install) -> Vec<String> {
    let mut path = ui::pretty(&install.path);
    if install.kind == InstallKind::Located {
        path.push_str(&style(" (located)").dim().to_string());
    }
    if let Some(name) =
        install.releases.as_deref().and_then(|url| launcher.settings.source_name(url, &launcher.official()))
    {
        path.push_str(&style(format!(" (from {name})")).dim().to_string());
    }
    if install.missing {
        path.push_str(&style(" (folder not found)").yellow().to_string());
    }
    let modules: Vec<&str> =
        Module::ALL.into_iter().filter(|module| install.has_module(*module)).map(Module::name).collect();
    let modules = style(modules.join(", ")).dim().to_string();
    vec![style(&install.version).bold().to_string(), install.channel.to_string(), modules, path]
}

fn version_of(row: &[String]) -> Option<Version> {
    Version::parse(&console::strip_ansi_codes(&row[0])).ok()
}

pub fn install(
    launcher: &Launcher,
    version: Option<String>,
    file: Option<PathBuf>,
    without: Vec<Module>,
    from: Option<&str>,
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

    let version = version.expect("clap requires a version or --file");
    let selector: Selector = version.parse()?;
    let catalog = launcher.catalog()?;
    let platform = launcher.store.platform();
    let listed = catalog.select(&selector, platform, from).map_err(|error| which_list(error, &version))?;
    let in_list = from.map(|from| format!(" in {from}'s releases")).unwrap_or_default();
    let listed = listed.ok_or_else(|| anyhow!("no release matches {selector} for {platform}{in_list}"))?;
    let release = listed.release;
    if let Some(install) = launcher.store.get(&release.version)? {
        ui::done(&format!("Voxear {} is already installed, at {}", install.version, ui::pretty(&install.path)));
        return Ok(());
    }

    let cancel = AtomicBool::new(false);
    let keys = &listed.source.keys;
    let install =
        launcher.store.install_without(release, listed.url, keys, &without, &mut |p| bar.update(p), &cancel)?;
    bar.finish();
    let with: Vec<&str> =
        Module::ALL.into_iter().filter(|module| install.has_module(*module)).map(Module::title).collect();
    let with = if with.is_empty() { String::new() } else { format!(", with the {}", with.join(" and ")) };
    let from = if listed.source.official { String::new() } else { format!(" from {}", listed.source.name) };
    ui::done(&format!(
        "Installed Voxear {} ({}{with}){from} in {}",
        install.version,
        install.channel,
        ui::pretty(&install.path)
    ));
    for module in without.into_iter().filter(|module| release.module(*module, platform).is_some()) {
        ui::hint(&format!("Add the {module} later with `voxup {} add {}`.", module.name(), install.version));
    }
    Ok(())
}

#[derive(clap::Subcommand)]
pub enum ModuleCommand {
    /// Download and add it to an installed version.
    Add { version: String },
    /// Remove it from an installed version, keeping the engine.
    Remove { version: String },
}

/// Adds or removes an optional part of an install, such as the manual or the Dedicated Server.
pub fn module(launcher: &Launcher, module: Module, command: ModuleCommand) -> Result<()> {
    match command {
        ModuleCommand::Add { version } => {
            let version = exact(&version)?;
            let install = launcher.store.get(&version)?.ok_or_else(|| anyhow!("Voxear {version} isn't installed"))?;
            let catalog = launcher.catalog()?;
            let listed = catalog.of_install(&install).ok_or_else(|| {
                anyhow!("Voxear {version} isn't in the release lists, so its {module} can't be found")
            })?;
            let (release, keys) = (listed.release, &listed.source.keys);
            let mut bar = InstallBar::new();
            let cancel = AtomicBool::new(false);
            launcher.store.add_module(release, module, listed.url, keys, &mut |p| bar.update(p), &cancel)?;
            bar.finish();
            ui::done(&format!("Added the {module} to Voxear {version}"));
        }
        ModuleCommand::Remove { version } => {
            let version = exact(&version)?;
            launcher.store.remove_module(&version, module)?;
            ui::done(&format!("Removed the {module} from Voxear {version}"));
        }
    }
    Ok(())
}

/// Says how to choose when two release lists publish different engines under the version asked for.
pub fn which_list(error: anyhow::Error, version: &str) -> anyhow::Error {
    let Some(ambiguous) = error.downcast_ref::<Ambiguous>() else { return error };
    let quote = |name: &str| if name.contains(' ') { format!("\"{name}\"") } else { name.to_owned() };
    let commands: Vec<_> =
        ambiguous.sources.iter().map(|name| format!("`voxup install {version} --from {}`", quote(name))).collect();
    anyhow!("{ambiguous}; install one with {}", commands.join(" or "))
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
