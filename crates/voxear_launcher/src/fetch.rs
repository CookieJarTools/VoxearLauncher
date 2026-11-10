//! Reads manifests and downloads from `https://` URLs or from local files (`file://` URLs and plain paths), so
//! a release mirror, an offline folder and the tests all go through the same code. Plain `http://` is allowed for
//! this machine only (`localhost`, `127.0.0.1`, `[::1]`), for trying releases served from a local folder.

use std::fmt;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const USER_AGENT: &str = concat!("voxup/", env!("CARGO_PKG_VERSION"));

pub struct Fetched {
    pub reader: Box<dyn Read + Send>,
    /// The size, when the source knows it.
    pub len: Option<u64>,
}

/// The source doesn't exist: a 404, or a missing file. Callers use it to tell "nothing has been published
/// there yet" apart from a failure.
#[derive(Debug)]
pub struct NotFound(pub String);

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "there's nothing at {}", self.0)
    }
}

impl std::error::Error for NotFound {}

/// Whether `error` comes from a source that doesn't exist.
pub fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<NotFound>())
}

/// Opens `source` for reading.
pub fn open(source: &str) -> Result<Fetched> {
    match Source::parse(source)? {
        Source::File(path) => {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == ErrorKind::NotFound => return Err(NotFound(path.display().to_string()).into()),
                Err(e) => return Err(e).with_context(|| format!("couldn't open {}", path.display())),
            };
            let len = file.metadata().ok().map(|meta| meta.len());
            Ok(Fetched { reader: Box::new(file), len })
        }
        Source::Web(url) => {
            let response = match agent().get(&url).call() {
                Ok(response) => response,
                Err(ureq::Error::StatusCode(404)) => return Err(NotFound(url).into()),
                Err(e) => bail!("couldn't download {url}: {e}"),
            };
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
    Web(String),
}

impl Source {
    fn parse(source: &str) -> Result<Self> {
        match source.split_once("://") {
            None => Ok(Source::File(PathBuf::from(source))),
            Some(("file", rest)) => Ok(Source::File(file_url_path(rest))),
            Some(("https", _)) => Ok(Source::Web(source.to_owned())),
            Some(("http", _)) if is_loopback(source) => Ok(Source::Web(source.to_owned())),
            Some(_) => bail!("{source}: only https:// URLs and local files are supported"),
        }
    }
}

/// Checks that `url` is one the launcher downloads from: `https://`, or `http://` on this machine.
pub fn check_web_url(url: &str) -> Result<()> {
    match url.split_once("://") {
        Some(("https", _)) => Ok(()),
        Some(("http", _)) if is_loopback(url) => Ok(()),
        _ => bail!("{url}: only https:// URLs are supported"),
    }
}

/// Whether an `http://` or `https://` URL points at this machine.
pub fn is_loopback(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once(']').map_or(bracketed, |(host, _)| host),
        None => host.split(':').next().unwrap_or_default(),
    };
    matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
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
        // A host that only starts like a loopback one is still somewhere else.
        assert!(open("http://127.0.0.1.example.invalid/releases.toml").is_err());
        assert!(open("http://localhost@example.invalid/releases.toml").is_err());
    }

    #[test]
    fn allows_plain_http_on_this_machine_only() {
        assert!(is_loopback("http://127.0.0.1:8080/launcher/stable.json"));
        assert!(is_loopback("http://localhost/stable.json"));
        assert!(is_loopback("http://[::1]:9000/stable.json"));
        assert!(is_loopback("http://user@127.0.0.1:9000/stable.json"));
        assert!(!is_loopback("http://127.0.0.1.example.invalid/stable.json"));
        assert!(!is_loopback("http://localhost@example.invalid/stable.json"));
        assert!(!is_loopback("http://10.0.0.1/stable.json"));
        assert!(!is_loopback("/srv/stable.json"));
        check_web_url("https://example.invalid/a").unwrap();
        check_web_url("http://127.0.0.1:8080/a").unwrap();
        assert!(check_web_url("http://example.invalid/a").is_err());
        assert!(check_web_url("ftp://127.0.0.1/a").is_err());
    }

    #[test]
    fn reads_over_http_from_this_machine() {
        let url = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        assert_eq!(read(&url, 100).unwrap(), b"hello");
    }

    #[test]
    fn a_download_cut_off_part_way_is_an_error() {
        // The server promises 100 bytes, sends 5 and hangs up.
        let url = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nhello");
        assert!(read(&url, 1000).is_err());
    }

    /// Answers one request on a loopback port with `response`, then closes the connection.
    fn serve_once(response: &'static [u8]) -> String {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            _ = stream.read(&mut request);
            _ = stream.write_all(response);
        });
        format!("http://127.0.0.1:{port}/file")
    }

    #[test]
    fn says_when_a_source_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let error = read(&dir.path().join("nothing.toml").to_string_lossy(), 10).err().unwrap();
        assert!(is_not_found(&error), "{error}");
        assert!(!is_not_found(&anyhow::anyhow!("something else")));
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
