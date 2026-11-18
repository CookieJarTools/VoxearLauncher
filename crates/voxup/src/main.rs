//! `voxup`: the Voxear Launcher on the command line. Everything here is a thin layer over `voxear_launcher`,
//! which the launcher window uses too.

mod engines;
mod projects;
mod selfupdate;
mod sources;
mod ui;
mod upgrades;
mod whisk;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use voxear_launcher::fetch;
use voxear_launcher::sources::ListingState;
use voxear_launcher::{Catalog, Layout, Module, Platform, Registry, Settings, Source, Store, TrustedKeys};

#[derive(Parser)]
#[command(name = "voxup", version, about = "Install Voxear engine versions, and create and open projects")]
struct Cli {
    /// Where the official engine releases are listed, instead of their usual place: an https:// URL or a
    /// releases.toml path. Custom releases are still listed beside them.
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
        /// Leave out the Voxear Manual, which is otherwise installed with the engine when the release offers it.
        #[arg(long)]
        without_manual: bool,
        /// Leave out the Dedicated Server, the program server builds ship; without it they use the player.
        #[arg(long)]
        without_server: bool,
        /// Install from these releases, by name, such as custom releases (see `voxup releases`).
        #[arg(long, value_name = "NAME", conflicts_with = "file")]
        from: Option<String>,
    },
    /// Add the Voxear Manual to an installed version, or remove it.
    #[command(subcommand)]
    Manual(engines::ModuleCommand),
    /// Add the Dedicated Server, which server builds ship, to an installed version, or remove it.
    #[command(subcommand)]
    Server(engines::ModuleCommand),
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
    /// Where engine releases come from: the official ones, nightlies, and custom ones such as a fork's.
    Releases {
        #[command(subcommand)]
        command: Option<sources::ReleasesCommand>,
    },
    /// Whisk, the code editor Voxear opens scripts in: install, open or remove it.
    #[command(subcommand)]
    Whisk(whisk::WhiskCommand),
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
        Command::Install { version, file, without_manual, without_server, from } => {
            let left_out = [(without_manual, Module::Manual), (without_server, Module::Server)];
            let without = left_out.into_iter().filter(|(leave, _)| *leave).map(|(_, module)| module).collect();
            engines::install(&launcher, version, file, without, from.as_deref())
        }
        Command::Manual(command) => engines::module(&launcher, Module::Manual, command),
        Command::Server(command) => engines::module(&launcher, Module::Server, command),
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
        Command::Releases { command } => sources::run(&launcher, command.unwrap_or(sources::ReleasesCommand::List)),
        Command::Whisk(command) => whisk::run(&launcher, command),
        Command::Templates { engine } => projects::templates(&launcher, engine.as_deref()),
    }
}

/// What every command needs: where things are, the settings and the installs.
pub struct Launcher {
    pub layout: Layout,
    pub settings: Settings,
    pub store: Store,
    /// Where the official releases are listed, when `--releases` says.
    official: Option<String>,
}

impl Launcher {
    fn new(official: Option<String>) -> Result<Self> {
        let layout = Layout::from_env()?;
        let settings = Settings::load(&layout)?;
        let platform = Platform::current().ok_or_else(|| anyhow!("Voxear isn't built for this platform"))?;
        let store = Store::new(layout.clone(), platform);
        Ok(Self { layout, settings, store, official })
    }

    pub fn keys(&self) -> Result<TrustedKeys> {
        self.settings.trusted_keys()
    }

    pub fn registry(&self) -> Result<Registry> {
        Registry::load(self.layout.projects_file())
    }

    /// Where the official releases are listed.
    pub fn official(&self) -> String {
        self.official.clone().unwrap_or_else(voxear_launcher::settings::official_url)
    }

    /// Every release list: the official one, then custom ones.
    pub fn sources(&self) -> Result<Vec<Source>> {
        match &self.official {
            Some(url) => self.settings.sources_with(url),
            None => self.settings.sources(),
        }
    }

    /// Every release list, verified. Warns about a list that couldn't be read, or only from its saved copy, and
    /// fails when no list could be read at all.
    pub fn catalog(&self) -> Result<Catalog> {
        let mut catalog = Catalog::load(self.sources()?, &self.layout);
        let any = catalog.loaded().next().is_some();
        for listing in &catalog.listings {
            let source = &listing.source;
            let from = if source.official { String::new() } else { format!(" from {}", source.name) };
            match &listing.state {
                ListingState::Loaded(releases) => {
                    if let Some(reason) = &releases.stale {
                        ui::warn(&format!(
                            "couldn't check for new releases{from}, so this is the list from last time: {reason}"
                        ));
                    }
                }
                ListingState::Failed(error) if any || !source.official => {
                    let which = if source.official {
                        "the official releases".to_owned()
                    } else {
                        format!("the releases{from}")
                    };
                    ui::warn(&format!("couldn't read {which}: {error:#}"))
                }
                _ => {}
            }
        }
        if !any {
            let official = catalog.listings.remove(0);
            return Err(match official.state {
                ListingState::Failed(error) => error,
                _ => anyhow::Error::new(fetch::NotFound(official.source.url))
                    .context("no engine releases are published yet"),
            });
        }
        Ok(catalog)
    }
}
