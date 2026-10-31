//! Signature checks for the release manifest and for every download. Voxear signs with minisign keys
//! (Ed25519 over a BLAKE2b pre-hash), so releases can be signed with the `minisign` tool, and the launcher's
//! own updates with the same kind of key Tauri's updater uses.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use minisign_verify::{Error as MinisignError, PublicKey, Signature};

/// The public keys engine releases are signed with, built into the launcher. A signature names the key that
/// made it, so a new key can be added here before the old one is retired.
pub const ENGINE_RELEASE_KEYS: &[&str] = &[
    // The engine release key, 4B25CB4CB88433F4, made 2026-10-31.
    "RWT0M4S4TMslS1Lg4k5jY4KJ8dMqEKipBFRlPzS5M0HS2+jtghspk77f",
];

#[derive(Clone, Debug, Default)]
pub struct TrustedKeys {
    keys: Vec<PublicKey>,
}

impl TrustedKeys {
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in engine release keys plus any extra ones, such as keys from the settings for self-hosted
    /// engine builds.
    pub fn engine_releases<'a>(extra: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let mut keys = Self::new();
        for key in ENGINE_RELEASE_KEYS.iter().copied().chain(extra) {
            keys.add(key)?;
        }
        Ok(keys)
    }

    /// Trusts a key, given either as the base64 line or as a whole minisign `.pub` file.
    pub fn add(&mut self, key: &str) -> Result<()> {
        let key = key.trim();
        let parsed = if key.contains('\n') { PublicKey::decode(key) } else { PublicKey::from_base64(key) };
        self.keys.push(parsed.map_err(|e| anyhow!("`{key}` isn't a minisign public key: {e}"))?);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn verify(&self, data: &[u8], signature: &str) -> Result<()> {
        self.verify_reader(data, signature)
    }

    pub fn verify_file(&self, path: &Path, signature: &str) -> Result<()> {
        let file = File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;
        self.verify_reader(file, signature)
    }

    /// Checks `signature` against everything `reader` yields, without holding it all in memory.
    pub fn verify_reader(&self, mut reader: impl Read, signature: &str) -> Result<()> {
        if self.keys.is_empty() {
            bail!("no signing keys are trusted yet, so nothing can be verified");
        }
        let signature = Signature::decode(signature.trim()).map_err(|e| anyhow!("the signature is malformed: {e}"))?;
        let key = self.key_for(&signature)?;
        let mut verifier = key.verify_stream(&signature).map_err(|e| anyhow!("can't check this signature: {e}"))?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = reader.read(&mut buffer).context("couldn't read what was signed")?;
            if read == 0 {
                break;
            }
            verifier.update(&buffer[..read]);
        }
        verifier.finalize().map_err(|_| anyhow!("the signature doesn't match; the file was changed after signing"))
    }

    fn key_for(&self, signature: &Signature) -> Result<&PublicKey> {
        for key in &self.keys {
            match key.verify_stream(signature) {
                Ok(_) => return Ok(key),
                Err(MinisignError::UnexpectedKeyId) => continue,
                Err(e) => bail!("can't check this signature: {e}"),
            }
        }
        bail!("it was signed with a key this launcher doesn't trust")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestKey;

    #[test]
    fn verifies_what_a_trusted_key_signed() {
        let key = TestKey::generate();
        let mut keys = TrustedKeys::new();
        keys.add(&key.public()).unwrap();
        keys.verify(b"voxear", &key.sign(b"voxear")).unwrap();
    }

    #[test]
    fn rejects_changed_data() {
        let key = TestKey::generate();
        let mut keys = TrustedKeys::new();
        keys.add(&key.public()).unwrap();
        let error = keys.verify(b"voxear!", &key.sign(b"voxear")).unwrap_err();
        assert!(error.to_string().contains("doesn't match"), "{error}");
    }

    #[test]
    fn rejects_signatures_from_unknown_keys() {
        let trusted = TestKey::generate();
        let stranger = TestKey::generate();
        let mut keys = TrustedKeys::new();
        keys.add(&trusted.public()).unwrap();
        let error = keys.verify(b"voxear", &stranger.sign(b"voxear")).unwrap_err();
        assert!(error.to_string().contains("doesn't trust"), "{error}");
    }

    #[test]
    fn finds_the_right_key_among_several() {
        let old = TestKey::generate();
        let new = TestKey::generate();
        let mut keys = TrustedKeys::new();
        keys.add(&old.public()).unwrap();
        keys.add(&new.public()).unwrap();
        keys.verify(b"voxear", &new.sign(b"voxear")).unwrap();
        keys.verify(b"voxear", &old.sign(b"voxear")).unwrap();
    }

    #[test]
    fn accepts_a_whole_public_key_file() {
        let key = TestKey::generate();
        let mut keys = TrustedKeys::new();
        keys.add(&format!("untrusted comment: minisign public key\n{}\n", key.public())).unwrap();
        keys.verify(b"voxear", &key.sign(b"voxear")).unwrap();
    }

    #[test]
    fn trusts_nothing_without_keys() {
        let key = TestKey::generate();
        assert!(TrustedKeys::new().verify(b"voxear", &key.sign(b"voxear")).is_err());
        assert!(TrustedKeys::new().add("not a key").is_err());
    }
}
