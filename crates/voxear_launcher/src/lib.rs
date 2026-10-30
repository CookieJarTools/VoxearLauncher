//! The Voxear Launcher core. Everything the launcher does lives here, behind an API that has no UI in it, so
//! the `voxup` command line and the launcher window drive exactly the same code.
//!
//! - [`manifest`]: the signed engine release manifest, and picking a release from it.
//! - [`store`]: engine versions installed side by side, verified before they are unpacked.
//! - [`registry`]: the project list, and which engine version each project needs.
//! - [`new_project`]: the new-project flow, which runs the chosen engine's own `vox new`.
//!
//! The formats are described in `docs/DESIGN.md`.

pub mod archive;
pub mod channel;
pub mod engine;
pub mod fetch;
pub mod fsutil;
pub mod layout;
pub mod manifest;
pub mod new_project;
pub mod platform;
pub mod project;
pub mod registry;
pub mod releases;
pub mod settings;
pub mod signing;
pub mod store;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use channel::{Channel, Selector};
pub use layout::Layout;
pub use manifest::{Download, Release, ReleaseManifest};
pub use platform::Platform;
pub use registry::{Project, ProjectStatus, Registry};
pub use settings::Settings;
pub use signing::TrustedKeys;
pub use store::{Install, InstallKind, Progress, Removed, Store};

/// This launcher's version. Releases that need a newer launcher say so in the manifest.
pub fn launcher_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is valid semver")
}
