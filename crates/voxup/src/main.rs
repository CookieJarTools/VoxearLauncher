//! `voxup`: the Voxear Launcher on the command line. Everything here is a thin layer over `voxear_launcher`,
//! which the launcher window uses too.

mod engines;
mod projects;
mod selfupdate;
mod ui;
mod upgrades;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use voxear_launcher::fetch;
use voxear_launcher::releases::{self, Releases};
use voxear_launcher::{Layout, Platform, Registry, Settings, Store, TrustedKeys};

#[derive(Parser)]
#[command(name = "voxup", version, about = "Install Voxear engine versions, and create and open projects")]
struct Cli {
    /// Where engine releases are listed, instead of the settings: an https:// URL or a releases.toml path.
    #[arg(long, global = true, value_name = "URL")]
    releases: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List installed engine versions.
    List {
        /// Also list versions that can be installed.
        #[arg(long, short)]
        available: bool,
    },
    /// Install an engine version: `stable`, `beta`, `nightly`, `0.3` or `0.3.1`.
    Install {
        #[arg(required_unless_present = "file")]
        version: Option<String>,
        /// Install an engine archive from disk instead. Its signature must sit next to it, as <ARCHIVE>.minisig.
        #[arg(long, value_name = "ARCHIVE", conflicts_with = "version")]
        file: Option<PathBuf>,
    },
    /// Uninstall an engine version. An engine added with `locate` is forgotten, and its files are kept.
    Uninstall {
        version: String,
        /// Uninstall even if projects in the list use it.
        #[arg(long)]
        force: bool,
    },
    /// Add an engine installed somewhere else, such as one built from source.
    Locate { folder: PathBuf },
    /// List projects, with the engine version each one needs.
    Projects,
    /// Add a project folder to the list.
    Add { folder: PathBuf },
    /// Take a project off the list. Its files are not touched.
    Remove { folder: PathBuf },
    /// Open a project with the engine version it is pinned to.
    Open {
        #[arg(default_value = ".")]
        folder: PathBuf,
        /// Install the engine version first if it is missing.
        #[arg(long)]
        install: bool,
    },
    /// Create a project with an engine version's `vox new`.
    New(projects::NewArgs),
    /// Move a project to another engine version, backing it up first. Also pins a project that has none.
    Upgrade(upgrades::UpgradeArgs),
    /// List project backups, newest first.
    Backups {
        /// Only this project's backups.
        folder: Option<PathBuf>,
    },
    /// Restore a backup into a new folder beside its project. Nothing is overwritten.
    Restore {
        /// The backup's archive, as `voxup backups` shows it.
        backup: PathBuf,
    },
    /// The launcher's own updates.
    #[command(name = "self", subcommand)]
    SelfCommand(selfupdate::SelfCommand),
    /// Show the templates, setup and IDEs an engine version offers for new projects.
    Templates {
        /// The engine version to ask (default: the newest stable one installed).
        #[arg(long, value_name = "VERSION")]
        engine: Option<String>,
    },
}

fn main() -> ExitCode {
    if let Ok(exe) = std::env::current_exe() {
        voxear_launcher::self_update::clean_up_after_update(&exe);
    }
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            ui::error(&error);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let launcher = Launcher::new(cli.releases)?;
    match cli.command {
        Command::List { available } => engines::list(&launcher, available),
        Command::Install { version, file } => engines::install(&launcher, version, file),
        Command::Uninstall { version, force } => engines::uninstall(&launcher, &version, force),
        Command::Locate { folder } => engines::locate(&launcher, &folder),
        Command::Projects => projects::list(&launcher),
        Command::Add { folder } => projects::add(&launcher, &folder),
        Command::Remove { folder } => projects::remove(&launcher, &folder),
        Command::Open { folder, install } => projects::open(&launcher, &folder, install),
        Command::New(args) => projects::new(&launcher, args),
        Command::Upgrade(args) => upgrades::upgrade(&launcher, args),
        Command::Backups { folder } => upgrades::backups(&launcher, folder.as_deref()),
        Command::Restore { backup } => upgrades::restore(&launcher, &backup),
        Command::SelfCommand(command) => selfupdate::run(&launcher, command),
        Command::Templates { engine } => projects::templates(&launcher, engine.as_deref()),
    }
}

/// What every command needs: where things are, the settings and the installs.
pub struct Launcher {
    pub layout: Layout,
    pub settings: Settings,
    pub store: Store,
    releases_url: Option<String>,
}

impl Launcher {
    fn new(releases_url: Option<String>) -> Result<Self> {
        let layout = Layout::from_env()?;
        let settings = Settings::load(&layout)?;
        let platform = Platform::current().ok_or_else(|| anyhow!("Voxear isn't built for this platform"))?;
        let store = Store::new(layout.clone(), platform);
        Ok(Self { layout, settings, store, releases_url })
    }

    pub fn keys(&self) -> Result<TrustedKeys> {
        self.settings.trusted_keys()
    }

    pub fn registry(&self) -> Result<Registry> {
        Registry::load(self.layout.projects_file())
    }

    /// The release manifest, verified. Warns when only the saved copy could be used.
    pub fn releases(&self) -> Result<Releases> {
        let url = self.releases_url.clone().or_else(|| self.settings.releases_url()).with_context(|| {
            format!(
                "no release source is set yet; set `releases` in {}, set VOXUP_RELEASES, or pass --releases",
                ui::pretty(&self.layout.settings_file())
            )
        })?;
        let releases = releases::load(&url, &self.keys()?, &self.layout).map_err(|error| {
            if fetch::is_not_found(&error) { error.context("no engine releases are published yet") } else { error }
        })?;
        if let Some(reason) = &releases.stale {
            ui::warn(&format!("couldn't check for new releases, so this is the list from last time: {reason}"));
        }
        Ok(releases)
    }
}
