//! Fetching the release manifest: download it and its signature, check the signature, and keep the last good
//! copy, so the launcher still knows what exists when it is offline.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::fetch;
use crate::fsutil;
use crate::layout::Layout;
use crate::manifest::{ReleaseManifest, SIGNATURE_SUFFIX};
use crate::signing::TrustedKeys;
use crate::store::hex;

const MAX_MANIFEST: u64 = 16 << 20;
const MAX_SIGNATURE: u64 = 4 << 10;

pub struct Releases {
    pub manifest: ReleaseManifest,
    /// Where the manifest came from. Download URLs may be relative to it.
    pub url: String,
    /// Set when the saved copy was used instead, saying why the manifest couldn't be fetched.
    pub stale: Option<String>,
}

/// Fetches and verifies the manifest at `url`, falling back to the last verified copy if that fails.
pub fn load(url: &str, keys: &TrustedKeys, layout: &Layout) -> Result<Releases> {
    match fetch_verified(url, keys, layout) {
        Ok(manifest) => Ok(Releases { manifest, url: url.to_owned(), stale: None }),
        Err(error) => match cached(url, keys, layout) {
            Some(manifest) => Ok(Releases { manifest, url: url.to_owned(), stale: Some(format!("{error:#}")) }),
            None => Err(error),
        },
    }
}

/// Fetches and verifies the manifest at `url`, without falling back to a saved copy.
pub fn fetch_verified(url: &str, keys: &TrustedKeys, layout: &Layout) -> Result<ReleaseManifest> {
    let text = fetch::read(url, MAX_MANIFEST)?;
    let signature = fetch::read(&format!("{url}{SIGNATURE_SUFFIX}"), MAX_SIGNATURE)
        .context("couldn't fetch the release manifest's signature")?;
    let signature = String::from_utf8(signature).context("the release manifest's signature isn't text")?;
    keys.verify(&text, &signature).context("the release manifest failed its signature check")?;
    let manifest = ReleaseManifest::parse(std::str::from_utf8(&text).context("the release manifest isn't text")?)?;

    if let Some(seen) = cached(url, keys, layout)
        && manifest.generated < seen.generated
    {
        bail!(
            "the release manifest at {url} is older ({}) than one this launcher has already seen ({}), so it was \
             ignored; the server may be sending an old copy",
            manifest.generated,
            seen.generated
        );
    }
    let (manifest_cache, signature_cache) = cache_paths(layout, url);
    fsutil::write_atomic(&manifest_cache, &text)?;
    fsutil::write_atomic(&signature_cache, signature.as_bytes())?;
    Ok(manifest)
}

/// The last manifest verified from `url`, checked again in case the file was changed.
pub fn cached(url: &str, keys: &TrustedKeys, layout: &Layout) -> Option<ReleaseManifest> {
    let (manifest_cache, signature_cache) = cache_paths(layout, url);
    let text = std::fs::read(manifest_cache).ok()?;
    let signature = std::fs::read_to_string(signature_cache).ok()?;
    keys.verify(&text, &signature).ok()?;
    ReleaseManifest::parse(std::str::from_utf8(&text).ok()?).ok()
}

fn cache_paths(layout: &Layout, url: &str) -> (PathBuf, PathBuf) {
    let key = &hex(&Sha256::digest(url.as_bytes()))[..16];
    let manifest = layout.cache().join(format!("releases-{key}.toml"));
    let signature = layout.cache().join(format!("releases-{key}.toml{SIGNATURE_SUFFIX}"));
    (manifest, signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Channel;
    use crate::testing::FakeReleases;

    #[test]
    fn loads_a_signed_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.3.0", Channel::Stable);
        let url = fake.write();
        let layout = Layout::new(dir.path().join("home"));

        let releases = load(&url, &fake.trusted_keys(), &layout).unwrap();
        assert!(releases.stale.is_none());
        assert_eq!(releases.manifest.releases.len(), 1);
    }

    #[test]
    fn rejects_a_manifest_changed_after_signing() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.3.0", Channel::Stable);
        let url = fake.write();
        let text = std::fs::read_to_string(&url).unwrap().replace("0.3.0", "0.3.1");
        std::fs::write(&url, text).unwrap();

        let layout = Layout::new(dir.path().join("home"));
        let error = fetch_verified(&url, &fake.trusted_keys(), &layout).unwrap_err();
        assert!(format!("{error:#}").contains("signature"), "{error:#}");
    }

    #[test]
    fn rejects_a_manifest_signed_by_someone_else() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.3.0", Channel::Stable);
        let url = fake.write();
        let stranger = FakeReleases::new(dir.path().join("other"));

        let layout = Layout::new(dir.path().join("home"));
        assert!(fetch_verified(&url, &stranger.trusted_keys(), &layout).is_err());
    }

    #[test]
    fn refuses_to_go_back_to_an_older_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().join("home"));
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.3.0", Channel::Stable);
        let old = std::fs::read(fake.write()).unwrap();
        let old_signature = std::fs::read(format!("{}{SIGNATURE_SUFFIX}", fake.url())).unwrap();

        fake.add("0.4.0", Channel::Stable);
        fake.manifest.generated = fake.manifest.generated.checked_add(jiff::SignedDuration::from_hours(1)).unwrap();
        let url = fake.write();
        fetch_verified(&url, &fake.trusted_keys(), &layout).unwrap();

        std::fs::write(&url, old).unwrap();
        std::fs::write(format!("{url}{SIGNATURE_SUFFIX}"), old_signature).unwrap();
        let error = fetch_verified(&url, &fake.trusted_keys(), &layout).unwrap_err();
        assert!(error.to_string().contains("older"), "{error:#}");

        let releases = load(&url, &fake.trusted_keys(), &layout).unwrap();
        assert!(releases.stale.is_some(), "the newer saved copy is used instead");
        assert!(releases.manifest.find(&semver::Version::new(0, 4, 0)).is_some());
    }

    #[test]
    fn uses_the_saved_copy_when_offline() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().join("home"));
        let mut fake = FakeReleases::new(dir.path().join("mirror"));
        fake.add("0.3.0", Channel::Stable);
        let url = fake.write();
        load(&url, &fake.trusted_keys(), &layout).unwrap();

        std::fs::remove_dir_all(dir.path().join("mirror")).unwrap();
        let releases = load(&url, &fake.trusted_keys(), &layout).unwrap();
        assert!(releases.stale.is_some());
        assert_eq!(releases.manifest.releases.len(), 1);
    }
}
