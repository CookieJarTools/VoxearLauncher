//! Reads manifests and downloads from `https://` URLs or from local files (`file://` URLs and plain paths), so
//! a release mirror, an offline folder and the tests all go through the same code.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

pub const USER_AGENT: &str = concat!("voxup/", env!("CARGO_PKG_VERSION"));

pub struct Fetched {
    pub reader: Box<dyn Read + Send>,
    /// The size, when the source knows it.
    pub len: Option<u64>,
}

/// Opens `source` for reading.
pub fn open(source: &str) -> Result<Fetched> {
    match Source::parse(source)? {
        Source::File(path) => {
            let file = File::open(&path).with_context(|| format!("couldn't open {}", path.display()))?;
            let len = file.metadata().ok().map(|meta| meta.len());
            Ok(Fetched { reader: Box::new(file), len })
        }
        Source::Https(url) => {
            let response = agent().get(&url).call().map_err(|e| anyhow!("couldn't download {url}: {e}"))?;
            let len = response.body().content_length();
            Ok(Fetched { reader: Box::new(response.into_body().into_reader()), len })
        }
    }
}

/// Reads all of `source`, refusing anything larger than `limit` bytes.
pub fn read(source: &str, limit: u64) -> Result<Vec<u8>> {
    let fetched = open(source)?;
    let mut bytes = Vec::new();
    fetched.reader.take(limit + 1).read_to_end(&mut bytes).with_context(|| format!("couldn't read {source}"))?;
    if bytes.len() as u64 > limit {
        bail!("{source} is larger than the {limit} bytes expected");
    }
    Ok(bytes)
}

/// Resolves a download URL from a manifest against the manifest's own location, so a manifest can name its
/// archives relative to itself and be mirrored as one folder.
pub fn resolve(base: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_owned();
    }
    if base.contains("://") {
        let folder = base.rsplit_once('/').map_or(base, |(folder, _)| folder);
        return format!("{folder}/{url}");
    }
    let folder = Path::new(base).parent().unwrap_or(Path::new(""));
    folder.join(url).to_string_lossy().into_owned()
}

enum Source {
    File(PathBuf),
    Https(String),
}

impl Source {
    fn parse(source: &str) -> Result<Self> {
        match source.split_once("://") {
            None => Ok(Source::File(PathBuf::from(source))),
            Some(("file", rest)) => Ok(Source::File(file_url_path(rest))),
            Some(("https", _)) => Ok(Source::Https(source.to_owned())),
            Some(_) => bail!("{source}: only https:// URLs and local files are supported"),
        }
    }
}

/// The path in a `file://` URL. `file:///C:/x` on Windows is `C:/x`.
fn file_url_path(rest: &str) -> PathBuf {
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = path.as_bytes();
    let windows_drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    PathBuf::from(if windows_drive { &path[1..] } else { path })
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config =
            ureq::Agent::config_builder().user_agent(USER_AGENT).timeout_connect(Some(Duration::from_secs(20))).build();
        ureq::Agent::new_with_config(config)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_urls_against_the_manifest() {
        let base = "https://example.invalid/voxear/releases.toml";
        assert_eq!(resolve(base, "voxear-0.3.0.tar.zst"), "https://example.invalid/voxear/voxear-0.3.0.tar.zst");
        assert_eq!(resolve(base, "https://cdn.invalid/a.tar.zst"), "https://cdn.invalid/a.tar.zst");
        let local = resolve("/srv/voxear/releases.toml", "a.tar.zst");
        assert_eq!(PathBuf::from(local), Path::new("/srv/voxear").join("a.tar.zst"));
    }

    #[test]
    fn reads_file_urls_and_plain_paths() {
        assert_eq!(file_url_path("/srv/releases.toml"), PathBuf::from("/srv/releases.toml"));
        assert_eq!(file_url_path("/C:/releases.toml"), PathBuf::from("C:/releases.toml"));
        assert_eq!(file_url_path("localhost/srv/a"), PathBuf::from("/srv/a"));
    }

    #[test]
    fn refuses_plain_http() {
        let error = open("http://example.invalid/releases.toml").err().unwrap();
        assert!(error.to_string().contains("only https://"), "{error}");
    }

    #[test]
    fn caps_how_much_it_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        std::fs::write(&path, [0u8; 100]).unwrap();
        let source = path.to_string_lossy();
        assert_eq!(read(&source, 100).unwrap().len(), 100);
        assert!(read(&source, 99).is_err());
    }
}
