//! Writes a folder of fake, signed engine releases, so the launcher can be tried without real ones.
//!
//! ```text
//! cargo run -p voxear_launcher --features testing --example fake_release -- /tmp/voxear-mirror
//! ```
//!
//! It prints the settings that make `voxup` use and trust it.

use anyhow::{Context, Result};
use voxear_launcher::Channel;
use voxear_launcher::testing::FakeReleases;

fn main() -> Result<()> {
    let dir = std::env::args().nth(1).context("usage: fake_release <folder>")?;
    let dir = std::path::absolute(dir)?;
    let mut releases = FakeReleases::new(&dir);
    for (version, channel) in [
        ("0.1.0", Channel::Stable),
        ("0.2.0", Channel::Stable),
        ("0.3.0-beta.1", Channel::Beta),
        ("0.3.0-nightly.20261030", Channel::Nightly),
    ] {
        releases.add(version, channel);
        releases.add_manual(version);
        releases.add_server(version);
    }
    let manifest = releases.write();
    println!("Wrote fake releases to {}", dir.display());
    println!();
    println!("To try the launcher with them in place of the official releases, put this in settings.toml in the");
    println!("launcher's folder (or VOXUP_HOME):");
    println!();
    println!("trusted_keys = [{:?}]", releases.key.public());
    println!();
    println!("and set VOXUP_RELEASES={manifest}");
    println!();
    println!("To list them beside the official releases instead, as custom releases:");
    println!();
    println!("voxup releases add {manifest} --name Fake --key {}", releases.key.public());
    Ok(())
}
