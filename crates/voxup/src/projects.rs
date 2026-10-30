//! Projects: the list, opening, and creating new ones.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use console::style;
use voxear_launcher::new_project::{self, NewProject, Offer};
use voxear_launcher::project::{self, ProjectInfo};
use voxear_launcher::{Install, ProjectStatus, Selector};

use crate::Launcher;
use crate::ui::{self, InstallBar};

pub fn list(launcher: &Launcher) -> Result<()> {
    let installs = launcher.store.list()?;
    let projects = launcher.registry()?.projects(&installs);
    if projects.is_empty() {
        println!("No projects yet.");
        ui::hint("Create one with `voxup new <name>`, or add one with `voxup add <folder>`.");
        return Ok(());
    }
    let any_favourite = projects.iter().any(|project| project.entry.favourite);
    let rows: Vec<Vec<String>> = projects
        .iter()
        .map(|project| {
            let star = if project.entry.favourite { style("★").yellow().to_string() } else { " ".to_owned() };
            let engine = project.engine.as_ref().map_or_else(|| "—".to_owned(), ToString::to_string);
            let note = match &project.status {
                ProjectStatus::Ready => String::new(),
                ProjectStatus::EngineMissing(version) => {
                    style(format!("needs Voxear {version}: voxup install {version}")).yellow().to_string()
                }
                ProjectStatus::Unpinned => style("not pinned to an engine version").dim().to_string(),
                ProjectStatus::Missing => style("folder not found").yellow().to_string(),
                ProjectStatus::Unreadable(reason) => style(reason).red().to_string(),
            };
            let mut row =
                vec![style(&project.entry.name).bold().to_string(), engine, ui::pretty(&project.entry.path), note];
            if any_favourite {
                row.insert(0, star);
            }
            row
        })
        .collect();
    ui::table(&rows);
    Ok(())
}

pub fn add(launcher: &Launcher, folder: &Path) -> Result<()> {
    let mut registry = launcher.registry()?;
    let entry = registry.add(folder)?.clone();
    registry.save()?;
    ui::done(&format!("Added {} ({})", entry.name, ui::pretty(&entry.path)));
    Ok(())
}

pub fn remove(launcher: &Launcher, folder: &Path) -> Result<()> {
    let mut registry = launcher.registry()?;
    let entry =
        registry.find(folder).cloned().ok_or_else(|| anyhow!("{} isn't in the project list", folder.display()))?;
    registry.remove(folder);
    registry.save()?;
    ui::done(&format!("Took {} off the list; its files are untouched", entry.name));
    Ok(())
}

pub fn open(launcher: &Launcher, folder: &Path, install_missing: bool) -> Result<()> {
    let root = project::project_root(folder);
    let info = ProjectInfo::read(&root)?;
    let Some(version) = info.engine else {
        bail!(
            "{} isn't pinned to an engine version yet; add `engine = \"<version>\"` under [project] in its voxear.toml",
            info.name
        );
    };
    let install = match launcher.store.get(&version)? {
        Some(install) if install.missing => {
            bail!("Voxear {version} was at {}, which isn't there now", ui::pretty(&install.path))
        }
        Some(install) => install,
        None if install_missing => install_exact(launcher, &version.to_string())?,
        None => bail!(
            "{} needs Voxear {version}, which isn't installed; install it with `voxup install {version}`, or run \
             `voxup open --install`",
            info.name
        ),
    };
    project::open(&install, &root)?;
    let mut registry = launcher.registry()?;
    registry.touch(&root)?;
    registry.save()?;
    ui::done(&format!("Opening {} with Voxear {version}", info.name));
    Ok(())
}

fn install_exact(launcher: &Launcher, version: &str) -> Result<Install> {
    let selector: Selector = version.parse()?;
    let releases = launcher.releases()?;
    let platform = launcher.store.platform();
    let release = releases
        .manifest
        .select(&selector, platform)
        .ok_or_else(|| anyhow!("Voxear {version} isn't in the release list for {platform}"))?;
    let mut bar = InstallBar::new();
    let install = launcher.store.install(
        release,
        &releases.url,
        &launcher.keys()?,
        &mut |p| bar.update(p),
        &AtomicBool::new(false),
    )?;
    bar.finish();
    ui::done(&format!("Installed Voxear {}", install.version));
    Ok(install)
}

#[derive(Args)]
pub struct NewArgs {
    /// The project's name, which is also its folder name.
    name: String,
    /// The folder to create the project in (default: the current folder).
    #[arg(long, short)]
    location: Option<PathBuf>,
    /// The template: empty, 3d, 2d, first-person, third-person or top-down (see `voxup templates`).
    #[arg(long, short)]
    template: Option<String>,
    /// The engine version (default: the newest stable one installed).
    #[arg(long, short, value_name = "VERSION")]
    engine: Option<String>,
    /// The default setup to include, comma-separated, or `none` (default: the template's own).
    #[arg(long, value_name = "ITEMS")]
    setup: Option<String>,
    /// Set the project up for an IDE: rider, visual-studio or vscode.
    #[arg(long)]
    ide: Option<String>,
    /// Start a git repository with a .gitignore.
    #[arg(long)]
    git: bool,
    /// Switch modding on.
    #[arg(long)]
    modding: bool,
    /// The game's Steam App ID.
    #[arg(long, value_name = "ID")]
    steam_app_id: Option<u32>,
}

pub fn new(launcher: &Launcher, args: NewArgs) -> Result<()> {
    let install = engine_for_new(launcher, args.engine.as_deref())?;
    let offer = new_project::describe(&install)?;
    let location = match args.location {
        Some(location) => location,
        None => std::env::current_dir().context("couldn't find the current folder")?,
    };
    let template = args.template.unwrap_or_else(|| default_template(&offer));
    let mut project = NewProject::new(args.name, location, template);
    project.setup = args.setup.map(|items| {
        if items.trim() == "none" { Vec::new() } else { items.split(',').map(|s| s.trim().to_owned()).collect() }
    });
    project.ide = args.ide;
    project.git = args.git;
    project.modding = args.modding;
    project.steam_app_id = args.steam_app_id;
    project.check(Some(&offer))?;

    println!("Creating {} with Voxear {}", style(project.name.trim()).bold(), install.version);
    let path = new_project::create(&install, &project, &mut |step| ui::hint(&format!("• {step}")))?;
    let mut registry = launcher.registry()?;
    registry.add(&path)?;
    registry.save()?;
    ui::done(&format!("Created {} in {}", project.name.trim(), ui::pretty(&path)));
    ui::hint(&format!("Open it with `voxup open {}`", ui::pretty(&path)));
    Ok(())
}

pub fn templates(launcher: &Launcher, engine: Option<&str>) -> Result<()> {
    let install = engine_for_new(launcher, engine)?;
    let offer = new_project::describe(&install)?;
    println!("New projects with Voxear {}:", install.version);
    println!();
    println!("{}", style("Templates").bold());
    ui::table(
        &offer
            .templates
            .iter()
            .map(|t| vec![t.id.clone(), t.name.clone(), style(&t.description).dim().to_string()])
            .collect::<Vec<_>>(),
    );
    for (title, choices) in [("Default setup", &offer.setup), ("IDEs", &offer.ides)] {
        if choices.is_empty() {
            continue;
        }
        println!();
        println!("{}", style(title).bold());
        ui::table(&choices.iter().map(|c| vec![c.id.clone(), c.name.clone()]).collect::<Vec<_>>());
    }
    Ok(())
}

fn engine_for_new(launcher: &Launcher, engine: Option<&str>) -> Result<Install> {
    let selector: Selector = engine.unwrap_or("stable").parse()?;
    if let Some(install) = launcher.store.pick(&selector)? {
        return Ok(install);
    }
    match engine {
        Some(engine) => bail!("no installed engine version matches `{engine}`; see `voxup list`"),
        None => bail!("no stable engine version is installed; install one with `voxup install stable`"),
    }
}

/// `3d` when the engine offers it, as the template most people start from.
fn default_template(offer: &Offer) -> String {
    let id = offer.template("3d").or(offer.templates.first()).map(|t| t.id.clone());
    id.unwrap_or_else(|| "empty".to_owned())
}
