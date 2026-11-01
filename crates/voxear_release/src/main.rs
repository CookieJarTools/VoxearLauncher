//! `voxear-release`: signs Voxear engine and launcher releases, and writes the manifests the launcher reads.
//! The site folder it writes is published with GitHub Pages; the files it signs are uploaded as release assets.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use semver::Version;
use voxear_launcher::self_update::UpdateChannel;
use voxear_release::{Added, EngineRelease, Key, LauncherBuild, LauncherSignature, signature_path};

#[derive(Parser)]
#[command(name = "voxear-release", version, about = "Sign Voxear releases and write the manifests the launcher reads")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make a signing key pair: <PATH>.key, encrypted with a password, and <PATH>.pub.
    Keygen {
        path: PathBuf,
        /// Write both files base64-wrapped, as `tauri signer generate` does, for the launcher update key.
        #[arg(long)]
        tauri: bool,
    },
    /// Engine releases.
    #[command(subcommand)]
    Engine(EngineCommand),
    /// Launcher builds.
    #[command(subcommand)]
    Launcher(LauncherCommand),
}

#[derive(Subcommand)]
enum EngineCommand {
    /// Sign engine archives and list them in the release manifest. Each archive's engine.toml says its version,
    /// channel and platform; a module archive, such as voxear-<version>-manual.tar.zst, is listed on its version.
    Add {
        #[arg(required = true)]
        archives: Vec<PathBuf>,
        #[command(flatten)]
        site: SiteArgs,
        /// The release tag the archives are uploaded under (default: engine-v<version>).
        #[arg(long)]
        tag: Option<String>,
        /// Where release assets are downloaded from (default: the VoxearReleases repository's releases).
        #[arg(long, value_name = "URL")]
        download_base: Option<String>,
        /// A link to the release notes.
        #[arg(long, value_name = "URL")]
        notes: Option<String>,
        /// The oldest launcher that can install it.
        #[arg(long, value_name = "VERSION")]
        min_launcher: Option<Version>,
    },
    /// Pull a release: no channel or range picks it, but projects pinned to it still install it.
    Yank {
        version: Version,
        #[command(flatten)]
        site: SiteArgs,
        /// Put it back instead.
        #[arg(long)]
        undo: bool,
    },
    /// Remove old nightlies from the manifest.
    Prune {
        /// How many nightlies to keep.
        #[arg(long, default_value_t = 30)]
        keep: usize,
        #[command(flatten)]
        site: SiteArgs,
    },
}

#[derive(Subcommand)]
enum LauncherCommand {
    /// List a launcher build in its channel's update manifest.
    Add {
        file: PathBuf,
        /// The site folder.
        #[arg(long)]
        site: PathBuf,
        #[arg(long, default_value = "stable")]
        channel: UpdateChannel,
        #[arg(long)]
        version: Version,
        /// linux-x86_64 or windows-x86_64 for the window; voxup-linux-x86_64 or voxup-windows-x86_64 for voxup.
        #[arg(long)]
        target: String,
        /// The .sig `tauri build` wrote for it. Without one, it is signed here with --key.
        #[arg(long, conflicts_with = "key")]
        sig: Option<PathBuf>,
        /// The launcher update key, to sign it here.
        #[arg(long, required_unless_present = "sig")]
        key: Option<PathBuf>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, value_name = "URL")]
        download_base: Option<String>,
        /// What changed, in Markdown.
        #[arg(long)]
        notes: Option<String>,
    },
}

#[derive(clap::Args)]
struct SiteArgs {
    /// The site folder: engine/ and launcher/ go in it.
    #[arg(long)]
    site: PathBuf,
    /// The engine release key (its password comes from VOXEAR_SIGNING_PASSWORD, or is asked for).
    #[arg(long)]
    key: PathBuf,
}

impl SiteArgs {
    fn key(&self) -> Result<Key> {
        Key::load(&self.key, Key::password_from_env())
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Keygen { path, tauri } => {
            let made = voxear_release::keygen(&path, Key::password_from_env(), tauri)?;
            println!("Secret key: {} (keep it out of every repository)", made.secret.display());
            println!("Public key: {}", made.public.display());
            if tauri {
                println!("For LAUNCHER_UPDATE_KEY and tauri.conf.json: {}", made.public_tauri);
            } else {
                println!("For ENGINE_RELEASE_KEYS: {}", made.public_line);
            }
        }
        Command::Engine(EngineCommand::Add { archives, site, tag, download_base, notes, min_launcher }) => {
            let key = site.key()?;
            let details = EngineRelease { tag, download_base, notes, min_launcher };
            for added in voxear_release::add_archives(&site.site, &archives, &key, &details)? {
                match added {
                    Added::Engine(release) => println!("Listed Voxear {} ({})", release.version, release.channel),
                    Added::Module { version, module } => println!("Listed the {} for Voxear {version}", module.title),
                }
            }
            for archive in &archives {
                println!("  upload {} and {}", archive.display(), signature_path(archive).display());
            }
            println!(
                "Publish {}",
                voxear_release::engine_manifest_path(&site.site).parent().unwrap_or(&site.site).display()
            );
        }
        Command::Engine(EngineCommand::Yank { version, site, undo }) => {
            voxear_release::yank_engine(&site.site, &version, !undo, &site.key()?)?;
            println!("{} Voxear {version}", if undo { "Restored" } else { "Yanked" });
        }
        Command::Engine(EngineCommand::Prune { keep, site }) => {
            let removed = voxear_release::prune_nightlies(&site.site, keep, &site.key()?)?;
            println!("Removed {} nightlies", removed.len());
        }
        Command::Launcher(LauncherCommand::Add {
            file,
            site,
            channel,
            version,
            target,
            sig,
            key,
            tag,
            download_base,
            notes,
        }) => {
            let build = LauncherBuild { channel, version, target, tag, download_base, notes };
            let key = key.map(|path| Key::load(&path, Key::password_from_env())).transpose()?;
            let signature = match (&sig, &key) {
                (Some(sig), _) => LauncherSignature::TauriSig(sig),
                (None, Some(key)) => LauncherSignature::Sign(key),
                (None, None) => unreachable!("clap requires --sig or --key"),
            };
            let manifest = voxear_release::add_launcher(&site, &file, &build, signature)?;
            println!("Listed {} {} for {} in {}", file.display(), build.version, build.target, manifest.display());
        }
    }
    Ok(())
}
