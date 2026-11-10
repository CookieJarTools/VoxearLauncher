//! The launcher's own updates and, on Linux, its place in the app menu. The window's commands and the
//! `--update`, `--install` and `--uninstall` flags share this code.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use semver::Version;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use voxear_launcher::self_install::{Icon, Places};
use voxear_launcher::self_update::{self, PlatformUpdate, UpdateChannel, UpdateManifest};
use voxear_launcher::{fetch, launcher_version};

/// The icons the app menu shows, the same as the bundle's.
pub const ICONS: &[Icon] = &[
    Icon { size: 32, png: include_bytes!("../icons/32x32.png") },
    Icon { size: 64, png: include_bytes!("../icons/64x64.png") },
    Icon { size: 128, png: include_bytes!("../icons/128x128.png") },
    Icon { size: 256, png: include_bytes!("../icons/128x128@2x.png") },
    Icon { size: 512, png: include_bytes!("../icons/icon.png") },
];

pub fn icon_sizes() -> Vec<u32> {
    ICONS.iter().map(|icon| icon.size).collect()
}

/// An update that was found and not installed yet, with what its manifest lists for this platform's download.
pub struct Found {
    pub update: Update,
    pub entry: PlatformUpdate,
}

pub enum Lookup {
    NothingPublished,
    UpToDate(Version),
    Available(Found),
}

/// The AppImage this launcher runs from, on Linux. The AppImage runtime sets `APPIMAGE` to its path.
pub fn appimage() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Why this copy of the launcher can't update itself, if it can't.
pub fn why_no_self_update() -> Option<String> {
    if self_update::update_key().is_none() {
        return Some("This build has no launcher update key, so it can't check for updates.".to_owned());
    }
    if cfg!(debug_assertions) {
        return Some("Development builds don't update themselves.".to_owned());
    }
    if cfg!(target_os = "linux") && appimage().is_none() {
        return Some(
            "Only the AppImage updates itself. This copy was installed some other way, so update it that way."
                .to_owned(),
        );
    }
    None
}

/// Looks for a newer launcher in `channel`. The launcher reads the manifest itself first, so "nothing published
/// yet" isn't a failure; Tauri's updater then reads it again and checks the update.
pub async fn look_for_update<R: Runtime>(app: &AppHandle<R>, channel: UpdateChannel) -> Result<Lookup> {
    let url = self_update::manifest_url(channel);
    fetch::check_web_url(&url).context("the window reads launcher updates from a web address")?;
    let lookup = url.clone();
    let manifest = tauri::async_runtime::spawn_blocking(move || self_update::fetch_manifest(&lookup)).await??;
    let current = launcher_version();
    let Some(manifest) = manifest else { return Ok(Lookup::NothingPublished) };
    if !self_update::is_newer(&manifest.version, &current) {
        return Ok(Lookup::UpToDate(current));
    }

    let endpoint = url.parse().with_context(|| format!("{url} isn't a URL"))?;
    let updater = app.updater_builder().endpoints(vec![endpoint])?.build()?;
    let Some(update) = updater.check().await.context("couldn't check for updates")? else {
        return Ok(Lookup::UpToDate(current));
    };
    // The size and hash come from the same manifest Tauri's updater read the signature from.
    let announced: UpdateManifest =
        serde_json::from_value(update.raw_json.clone()).context("the launcher's update manifest isn't valid")?;
    let entry = announced
        .platforms
        .into_values()
        .find(|entry| entry.signature == update.signature)
        .ok_or_else(|| anyhow!("the update manifest changed while it was being read; check again"))?;
    fetch::check_web_url(update.download_url.as_str())?;
    Ok(Lookup::Available(Found { update, entry }))
}

/// Downloads an update into memory, checks its size, hash and signature, and only then installs it. On Linux
/// that replaces the AppImage, and the new version starts on the next restart. On Windows the setup program
/// takes over: the launcher closes, and the setup starts it again once it's done.
pub async fn install(found: Found, mut progress: impl FnMut(u64, Option<u64>) + Send) -> Result<()> {
    let mut done = 0u64;
    let bytes = found
        .update
        .download(
            |chunk, total| {
                done += chunk as u64;
                progress(done, total);
            },
            || {},
        )
        .await
        .context("couldn't download the update")?;
    found.entry.check_bytes(&bytes)?;
    found.update.install(&bytes).context("couldn't install the update")
}

/// Where the launcher is in the app menu, on Linux.
pub enum AppMenu {
    /// This is the installed copy.
    Installed(PathBuf),
    /// This is an AppImage that isn't installed, such as one just downloaded.
    NotInstalled,
    /// Windows, where the setup program installs the launcher, or a copy that isn't an AppImage.
    NotApplicable,
}

pub fn app_menu() -> AppMenu {
    let (Some(running), Ok(places)) = (appimage(), Places::from_env()) else { return AppMenu::NotApplicable };
    if places.is_installed_copy(&running) { AppMenu::Installed(places.program()) } else { AppMenu::NotInstalled }
}
