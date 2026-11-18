//! Where engine releases come from: the official releases, and any custom releases the settings add, such as a
//! fork of the engine's. Each one is a signed release manifest of its own, fetched and checked as [`releases`]
//! does. The engine list merges them, newest first, and says which list each release came from.
//!
//! An engine version is one install and one pin in a project's `voxear.toml`, so a version two lists publish as
//! different engines can't be told apart by its number: installing it means saying which list to take it from.
//! A list that mirrors another, with the same archives, isn't a clash: the release shows once.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};
use semver::Version;

use crate::channel::{Channel, Selector};
use crate::fetch;
use crate::layout::Layout;
use crate::manifest::{MANIFEST_FILE, Release};
use crate::platform::Platform;
use crate::releases::{self, Releases};
use crate::signing::TrustedKeys;
use crate::store::Install;

/// What the engine list calls the official releases.
pub const OFFICIAL: &str = "Official";

/// One release list, and the keys its manifest and archives must be signed with.
#[derive(Clone, Debug)]
pub struct Source {
    /// [`OFFICIAL`], or the name of custom releases.
    pub name: String,
    /// Where its manifest is: an `https://` URL or a local path.
    pub url: String,
    pub official: bool,
    pub keys: TrustedKeys,
}

/// One list's releases, as far as they could be read.
pub struct Listing {
    pub source: Source,
    pub state: ListingState,
}

pub enum ListingState {
    Loaded(Releases),
    /// Nothing is published there yet.
    Unpublished,
    Failed(anyhow::Error),
}

/// Every list's releases, the official ones first.
pub struct Catalog {
    pub listings: Vec<Listing>,
}

/// A release, and the list it came from.
#[derive(Clone, Copy)]
pub struct Listed<'a> {
    pub source: &'a Source,
    pub release: &'a Release,
    /// Where the list's manifest was read from, which download URLs may be relative to.
    pub url: &'a str,
}

/// Two lists publish different engines under one version.
#[derive(Debug)]
pub struct Ambiguous {
    pub version: Version,
    /// The lists that publish it, the official one first.
    pub sources: Vec<String>,
}

impl fmt::Display for Ambiguous {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = match self.sources.as_slice() {
            [] => "Two lists".to_owned(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        write!(f, "{names} both publish a Voxear {}, and they aren't the same engine", self.version)
    }
}

impl std::error::Error for Ambiguous {}

impl Catalog {
    /// Fetches and checks every list, falling back to each one's saved copy. A list that can't be read is kept
    /// with its error, so the others are still listed.
    pub fn load(sources: Vec<Source>, layout: &Layout) -> Self {
        let listings = sources
            .into_iter()
            .map(|source| {
                let state = match releases::load(&source.url, &source.keys, layout) {
                    Ok(loaded) => ListingState::Loaded(loaded),
                    Err(error) if fetch::is_not_found(&error) => ListingState::Unpublished,
                    Err(error) => ListingState::Failed(error),
                };
                Listing { source, state }
            })
            .collect();
        Self { listings }
    }

    /// Each list's last checked copy, without going online.
    pub fn cached(sources: Vec<Source>, layout: &Layout) -> Self {
        let listings = sources
            .into_iter()
            .map(|source| {
                let state = match releases::cached(&source.url, &source.keys, layout) {
                    Some(manifest) => ListingState::Loaded(Releases { manifest, url: source.url.clone(), stale: None }),
                    None => ListingState::Failed(anyhow::anyhow!(
                        "the releases of {} haven't been checked yet",
                        source.name
                    )),
                };
                Listing { source, state }
            })
            .collect();
        Self { listings }
    }

    /// The lists that could be read.
    pub fn loaded(&self) -> impl Iterator<Item = (&Source, &Releases)> {
        self.listings.iter().filter_map(|listing| match &listing.state {
            ListingState::Loaded(releases) => Some((&listing.source, releases)),
            _ => None,
        })
    }

    /// The list called `name`, ignoring case.
    pub fn listing(&self, name: &str) -> Option<&Listing> {
        self.listings.iter().find(|listing| listing.source.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Releases that can be installed on `platform`, from every list, newest first; a version published by more
    /// than one list comes official first, then in the settings' order. Yanked releases are left out, and so are
    /// copies of a release another list already has.
    pub fn available(&self, platform: Platform) -> Vec<Listed<'_>> {
        let mut listed: Vec<Listed> = Vec::new();
        for (source, releases) in self.loaded() {
            for release in releases.manifest.available(platform) {
                if !listed.iter().any(|known| same_engine(known.release, release, platform)) {
                    listed.push(Listed { source, release, url: &releases.url });
                }
            }
        }
        // Stable, so equal versions keep the lists' order.
        listed.sort_by(|a, b| b.release.version.cmp(&a.release.version));
        listed
    }

    /// The release of exactly `version` for `platform`, from the list called `from` when it is given. Yanked
    /// releases are found too, so projects pinned to one keep opening. Fails with [`Ambiguous`] when lists publish
    /// different engines under that version and `from` doesn't say which.
    pub fn find(&self, version: &Version, platform: Platform, from: Option<&str>) -> Result<Option<Listed<'_>>> {
        let mut found: Vec<Listed> = Vec::new();
        for (source, releases) in self.from(from)? {
            let Some(release) = releases.manifest.find(version).filter(|release| release.download(platform).is_some())
            else {
                continue;
            };
            if !found.iter().any(|known| same_engine(known.release, release, platform)) {
                found.push(Listed { source, release, url: &releases.url });
            }
        }
        if found.len() > 1 {
            let sources = found.iter().map(|listed| listed.source.name.clone()).collect();
            return Err(Ambiguous { version: version.clone(), sources }.into());
        }
        Ok(found.pop())
    }

    /// The release `selector` picks for `platform`. An exact version is found in every list, as [`Catalog::find`]
    /// does. A channel or a range picks from the official releases, or from the first list after them that has a
    /// match, unless `from` names the list.
    pub fn select(&self, selector: &Selector, platform: Platform, from: Option<&str>) -> Result<Option<Listed<'_>>> {
        if let Selector::Exact(version) = selector {
            return self.find(version, platform, from);
        }
        Ok(self.from(from)?.find_map(|(source, releases)| {
            let release = releases.manifest.select(selector, platform)?;
            Some(Listed { source, release, url: &releases.url })
        }))
    }

    /// The release an installed engine came from, for adding its optional parts: from the list it was installed
    /// from, when that is known and still listed, otherwise the first list that has its version.
    pub fn of_install(&self, install: &Install) -> Option<Listed<'_>> {
        let in_list = |url: Option<&str>| {
            let mut lists = self.loaded().filter(|(source, _)| url.is_none_or(|url| same_place(&source.url, url)));
            lists.find_map(|(source, releases)| {
                let release = releases.manifest.find(&install.version)?;
                Some(Listed { source, release, url: &releases.url })
            })
        };
        install.releases.as_deref().and_then(|url| in_list(Some(url))).or_else(|| in_list(None))
    }

    fn from(&self, from: Option<&str>) -> Result<impl Iterator<Item = (&Source, &Releases)>> {
        if let Some(name) = from {
            let Some(listing) = self.listing(name) else {
                let names: Vec<_> = self.listings.iter().map(|listing| listing.source.name.as_str()).collect();
                bail!("there are no releases called {name}; the lists are {}", names.join(", "));
            };
            if let ListingState::Failed(error) = &listing.state {
                bail!("couldn't read the releases of {}: {error:#}", listing.source.name);
            }
        }
        Ok(self
            .loaded()
            .filter(move |(source, _)| from.is_none_or(|name| source.name.eq_ignore_ascii_case(name.trim()))))
    }

    /// How many releases in a channel the lists have for `platform`, such as nightlies left out of a list.
    pub fn count(&self, platform: Platform, channel: Channel) -> usize {
        self.available(platform).iter().filter(|listed| listed.release.channel == channel).count()
    }
}

/// Reads a list once, as adding it does, so a wrong address or key shows at once: how many releases it has, or
/// `None` when nothing is published there yet.
pub fn check(source: &Source, layout: &Layout) -> Result<Option<usize>> {
    match releases::fetch_verified(&source.url, &source.keys, layout) {
        Ok(manifest) => Ok(Some(manifest.releases.len())),
        Err(error) if fetch::is_not_found(&error) => Ok(None),
        Err(error) => Err(error.context(format!("couldn't read the releases at {}", source.url))),
    }
}

/// The same engine, published in two lists: the same version with the same archive for `platform`.
fn same_engine(a: &Release, b: &Release, platform: Platform) -> bool {
    a.version == b.version
        && match (a.download(platform), b.download(platform)) {
            (Some(a), Some(b)) => a.sha256.eq_ignore_ascii_case(&b.sha256),
            _ => false,
        }
}

/// The release manifest's address for an address someone gave: the manifest itself (an `https://` URL, a
/// `file://` URL or a path), the site or folder it is in, or the GitHub repository it is published from
/// (`https://github.com/owner/repo` or `owner/repo`). A repository's manifest is on its GitHub Pages site, under
/// `engine/`, as the release workflow publishes it; so is a site's.
pub fn manifest_address(address: &str) -> Result<String> {
    let address = address.trim();
    ensure!(!address.is_empty(), "the address is empty");
    if let Some((owner, repo)) = github_repo(address) {
        return Ok(format!("https://{}.github.io/{repo}/engine/{MANIFEST_FILE}", owner.to_ascii_lowercase()));
    }
    match address.split_once("://") {
        Some(("https", _)) => {
            if address.ends_with(".toml") {
                return Ok(address.to_owned());
            }
            let site = address.trim_end_matches('/');
            let engine = if site.ends_with("/engine") { "" } else { "/engine" };
            Ok(format!("{site}{engine}/{MANIFEST_FILE}"))
        }
        Some(("file", _)) | None => {
            let path = local_path(address);
            if !path.is_dir() {
                return Ok(address.to_owned());
            }
            let candidates = [path.join(MANIFEST_FILE), path.join("engine").join(MANIFEST_FILE)];
            let manifest = candidates.iter().find(|candidate| candidate.is_file()).unwrap_or(&candidates[0]);
            Ok(manifest.to_string_lossy().into_owned())
        }
        Some(_) => bail!("{address}: only https:// addresses and local files can be used"),
    }
}

/// Whether two addresses lead to the same manifest.
pub fn same_place(a: &str, b: &str) -> bool {
    let normal = |address: &str| {
        let address = manifest_address(address).unwrap_or_else(|_| address.trim().to_owned());
        // Hosts don't care about case; paths might, but a list given twice is rarely typed two ways.
        address.trim_end_matches('/').to_ascii_lowercase()
    };
    normal(a) == normal(b)
}

/// A name for releases at `address`, for when nobody gave one: the owner of a GitHub repository or Pages site,
/// a site's host, or a folder's name.
pub fn name_for(address: &str) -> String {
    let address = address.trim();
    if let Some((owner, _)) = github_repo(address) {
        return owner;
    }
    if let Some(rest) = address.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or(rest);
        if let Some(owner) = host.strip_suffix(".github.io") {
            return owner.to_owned();
        }
        return host.strip_prefix("www.").unwrap_or(host).to_owned();
    }
    let path = local_path(address);
    let folder =
        if path.extension().is_some_and(|extension| extension == "toml") { path.parent() } else { Some(&*path) };
    // The release tool writes a site with the manifest in `engine/`; the site's folder is the better name.
    let folder = match folder {
        Some(folder) if folder.file_name().is_some_and(|name| name == "engine") => folder.parent(),
        other => other,
    };
    folder
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Custom".to_owned())
}

/// The owner and repository of a GitHub repository's address: `https://github.com/owner/repo`, with anything after
/// it, or `owner/repo` when that isn't a path on this machine.
fn github_repo(address: &str) -> Option<(String, String)> {
    let path = address
        .strip_prefix("https://github.com/")
        .or_else(|| address.strip_prefix("github.com/"))
        .or_else(|| (!address.contains("://") && !Path::new(address).exists()).then_some(address))?;
    let shorthand = !address.contains("github.com/");
    let mut parts = path.trim_end_matches('/').split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if shorthand && parts.next().is_some() {
        return None;
    }
    let name = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            && part != "."
            && part != ".."
    };
    let looks_like_a_file = repo.ends_with(".toml");
    (name(owner) && name(repo) && !looks_like_a_file && !owner.starts_with('.'))
        .then(|| (owner.to_owned(), repo.to_owned()))
}

fn local_path(address: &str) -> PathBuf {
    PathBuf::from(address.strip_prefix("file://").unwrap_or(address))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{OFFICIAL_RELEASES, Settings};
    use crate::testing::FakeReleases;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn reads_the_addresses_releases_are_given_by() {
        let pages = "https://someone.github.io/VoxearFork/engine/releases.toml";
        for address in [
            "someone/VoxearFork",
            "https://github.com/someone/VoxearFork",
            "https://github.com/Someone/VoxearFork.git",
            "https://github.com/someone/VoxearFork/releases",
            "github.com/someone/VoxearFork/",
            "https://someone.github.io/VoxearFork",
            "https://someone.github.io/VoxearFork/engine/",
            pages,
        ] {
            assert_eq!(manifest_address(address).unwrap(), pages, "{address}");
        }
        assert_eq!(manifest_address(" https://example.org/r.toml ").unwrap(), "https://example.org/r.toml");
        assert!(manifest_address("http://example.org/releases.toml").is_err());
        assert!(manifest_address("ftp://example.org/").is_err());
        assert!(manifest_address("  ").is_err());
        assert_eq!(manifest_address(OFFICIAL_RELEASES).unwrap(), OFFICIAL_RELEASES);
    }

    #[test]
    fn finds_the_manifest_in_a_release_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mirror = dir.path().join("mirror");
        std::fs::create_dir_all(&mirror).unwrap();
        let folder = mirror.to_string_lossy();
        assert_eq!(manifest_address(&folder).unwrap(), mirror.join("releases.toml").to_string_lossy());

        let site = dir.path().join("site");
        std::fs::create_dir_all(site.join("engine")).unwrap();
        std::fs::write(site.join("engine/releases.toml"), "").unwrap();
        let expected = site.join("engine/releases.toml").to_string_lossy().into_owned();
        assert_eq!(manifest_address(&site.to_string_lossy()).unwrap(), expected);
        assert_eq!(manifest_address(&format!("file://{}", site.display())).unwrap(), expected);
        assert_eq!(name_for(&expected), "site");
        let file = mirror.join("releases.toml").to_string_lossy().into_owned();
        assert_eq!(manifest_address(&file).unwrap(), file, "a manifest's own path is kept");
    }

    #[test]
    fn tells_when_two_addresses_are_one_list() {
        assert!(same_place(OFFICIAL_RELEASES, "CookieJarTools/VoxearReleases"));
        assert!(same_place(OFFICIAL_RELEASES, "https://CookieJarTools.github.io/VoxearReleases/"));
        assert!(!same_place(OFFICIAL_RELEASES, "someone/VoxearReleases"));
    }

    /// Official releases and a fork's, each signed with its own key, and the settings that list the fork.
    struct Two {
        dir: tempfile::TempDir,
        official: FakeReleases,
        fork: FakeReleases,
    }

    impl Two {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let official = FakeReleases::new(dir.path().join("official"));
            let fork = FakeReleases::new(dir.path().join("fork"));
            Self { dir, official, fork }
        }

        fn settings(&self) -> Settings {
            let mut settings = Settings { trusted_keys: vec![self.official.key.public()], ..Settings::default() };
            settings.add_custom(&self.fork.url(), Some("Fork"), Some(&self.fork.key.public())).unwrap();
            settings
        }

        fn catalog(&self) -> Catalog {
            self.official.write();
            self.fork.write();
            let sources = self.settings().sources_with(&self.official.url()).unwrap();
            Catalog::load(sources, &Layout::new(self.dir.path().join("home")))
        }
    }

    fn listed(catalog: &Catalog) -> Vec<(String, String)> {
        let available = catalog.available(Platform::LinuxX86_64);
        available.iter().map(|listed| (listed.release.version.to_string(), listed.source.name.clone())).collect()
    }

    #[test]
    fn reads_custom_releases_signed_with_their_own_key() {
        let mut two = Two::new();
        two.fork.add("0.2.0+fork", Channel::Stable);
        two.fork.add("0.3.0-nightly.20261117+fork", Channel::Nightly);
        let catalog = two.catalog();
        let fork = catalog.listing("fork").unwrap();
        let ListingState::Loaded(releases) = &fork.state else { panic!("the fork's releases didn't load") };
        assert_eq!(releases.manifest.releases.len(), 2);
        assert!(matches!(catalog.listing("Official").unwrap().state, ListingState::Loaded(_)));
        assert_eq!(catalog.count(Platform::LinuxX86_64, Channel::Nightly), 1);
    }

    #[test]
    fn refuses_custom_releases_signed_with_another_key() {
        let mut two = Two::new();
        two.fork.add("0.2.0+fork", Channel::Stable);
        two.fork.write();
        let mut settings = Settings::default();
        let stranger = crate::testing::TestKey::generate().public();
        settings.add_custom(&two.fork.url(), Some("Fork"), Some(&stranger)).unwrap();
        let sources = settings.sources_with(&two.official.url()).unwrap();
        let catalog = Catalog::load(sources, &Layout::new(two.dir.path().join("home")));
        let ListingState::Failed(error) = &catalog.listing("Fork").unwrap().state else { panic!("it loaded") };
        assert!(format!("{error:#}").contains("signature"), "{error:#}");
        assert!(catalog.available(Platform::LinuxX86_64).is_empty());
    }

    #[test]
    fn does_not_trust_one_list_with_another_lists_key() {
        let mut two = Two::new();
        two.official.add("0.1.0", Channel::Stable);
        two.official.write();
        // The official releases, re-signed with the fork's key: only the fork's own list trusts that key.
        let text = std::fs::read(two.official.url()).unwrap();
        std::fs::write(format!("{}.minisig", two.official.url()), two.fork.key.sign(&text)).unwrap();
        let sources = two.settings().sources_with(&two.official.url()).unwrap();
        let catalog = Catalog::load(sources, &Layout::new(two.dir.path().join("home")));
        assert!(matches!(catalog.listing("Official").unwrap().state, ListingState::Failed(_)));
    }

    #[test]
    fn merges_the_lists_newest_first_saying_where_each_came_from() {
        let mut two = Two::new();
        two.official.add("0.1.0", Channel::Stable);
        two.official.add("0.2.0", Channel::Stable);
        two.fork.add("0.2.0+fork", Channel::Stable);
        two.fork.add("0.1.5", Channel::Stable);
        let catalog = two.catalog();
        let expected = [("0.2.0+fork", "Fork"), ("0.2.0", "Official"), ("0.1.5", "Fork"), ("0.1.0", "Official")];
        let expected: Vec<_> = expected.iter().map(|(v, s)| (v.to_string(), s.to_string())).collect();
        assert_eq!(listed(&catalog), expected);
    }

    #[test]
    fn keeps_listing_the_others_when_one_list_fails() {
        let mut two = Two::new();
        two.official.add("0.1.0", Channel::Stable);
        let mut settings = two.settings();
        settings.add_custom("https://nowhere.invalid/releases.toml", Some("Gone"), None).unwrap();
        two.official.write();
        let sources = settings.sources_with(&two.official.url()).unwrap();
        let catalog = Catalog::load(sources, &Layout::new(two.dir.path().join("home")));
        assert!(matches!(catalog.listing("Gone").unwrap().state, ListingState::Failed(_)));
        assert!(matches!(catalog.listing("Fork").unwrap().state, ListingState::Unpublished), "nothing written yet");
        assert_eq!(listed(&catalog), [("0.1.0".to_owned(), "Official".to_owned())]);
    }

    #[test]
    fn shows_a_mirrored_release_once() {
        let mut two = Two::new();
        two.official.add("0.1.0", Channel::Stable);
        two.official.write();
        // A mirror of the official releases: the same archives under the same signatures.
        let mirror = two.dir.path().join("mirror");
        std::fs::create_dir_all(&mirror).unwrap();
        for entry in std::fs::read_dir(&two.official.dir).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), mirror.join(entry.file_name())).unwrap();
        }
        let mut settings = Settings { trusted_keys: vec![two.official.key.public()], ..Settings::default() };
        settings.add_custom(&mirror.to_string_lossy(), Some("Mirror"), None).unwrap();
        let sources = settings.sources_with(&two.official.url()).unwrap();
        let catalog = Catalog::load(sources, &Layout::new(two.dir.path().join("home")));
        assert_eq!(listed(&catalog), [("0.1.0".to_owned(), "Official".to_owned())]);
        let found = catalog.find(&v("0.1.0"), Platform::LinuxX86_64, None).unwrap().unwrap();
        assert_eq!(found.source.name, "Official");
    }

    #[test]
    fn finds_a_pinned_version_in_whichever_list_has_it() {
        let mut two = Two::new();
        two.official.add("0.1.0", Channel::Stable);
        two.fork.add("0.2.0+fork", Channel::Stable);
        let catalog = two.catalog();
        let found = catalog.find(&v("0.2.0+fork"), Platform::LinuxX86_64, None).unwrap().unwrap();
        assert_eq!(found.source.name, "Fork");
        assert_eq!(found.url, two.fork.url());
        let found = catalog.find(&v("0.1.0"), Platform::LinuxX86_64, None).unwrap().unwrap();
        assert_eq!(found.source.name, "Official");
        assert!(catalog.find(&v("0.9.0"), Platform::LinuxX86_64, None).unwrap().is_none());
        assert!(catalog.find(&v("0.1.0"), Platform::LinuxX86_64, Some("Fork")).unwrap().is_none());
        assert!(catalog.find(&v("0.1.0"), Platform::LinuxX86_64, Some("Nobody")).is_err());
    }

    #[test]
    fn asks_which_list_when_two_publish_different_engines_as_one_version() {
        let mut two = Two::new();
        two.official.add("0.2.0-nightly.20261117", Channel::Nightly);
        two.fork.add("0.2.0-nightly.20261117", Channel::Nightly);
        // Fake engines of one version are the same bytes; a fork's build of that day differs.
        let platform = Platform::LinuxX86_64;
        let version = v("0.2.0-nightly.20261117");
        let mut archive = crate::testing::engine_archive(&version, Channel::Nightly, platform);
        archive.extend_from_slice(b"fork");
        two.fork.add_archive(&version, Channel::Nightly, platform, &archive);
        let catalog = two.catalog();

        assert_eq!(listed(&catalog).len(), 2, "both are listed");
        let error = catalog.find(&version, platform, None).err().unwrap();
        let ambiguous = error.downcast_ref::<Ambiguous>().unwrap();
        assert_eq!(ambiguous.sources, ["Official", "Fork"]);
        assert!(error.to_string().contains("Official and Fork both publish"), "{error}");
        let picked = catalog.find(&version, platform, Some("fork")).unwrap().unwrap();
        assert_eq!(picked.source.name, "Fork");
    }

    #[test]
    fn picks_channels_and_ranges_from_the_official_releases_first() {
        let mut two = Two::new();
        two.official.add("0.2.0", Channel::Stable);
        two.fork.add("0.2.0+fork", Channel::Stable);
        two.fork.add("0.3.0-beta.1+fork", Channel::Beta);
        let catalog = two.catalog();
        let pick = |selector: &str, from: Option<&str>| {
            let listed = catalog.select(&selector.parse().unwrap(), Platform::LinuxX86_64, from).unwrap();
            listed.map(|listed| (listed.release.version.to_string(), listed.source.name.clone()))
        };
        assert_eq!(pick("stable", None), Some(("0.2.0".to_owned(), "Official".to_owned())));
        assert_eq!(pick("stable", Some("Fork")), Some(("0.2.0+fork".to_owned(), "Fork".to_owned())));
        assert_eq!(
            pick("beta", None),
            Some(("0.3.0-beta.1+fork".to_owned(), "Fork".to_owned())),
            "only the fork has one"
        );
        assert_eq!(pick("0.2.0+fork", None), Some(("0.2.0+fork".to_owned(), "Fork".to_owned())));
    }

    #[test]
    fn finds_an_installs_release_in_the_list_it_came_from() {
        let mut two = Two::new();
        two.official.add("0.1.0", Channel::Stable);
        two.fork.add("0.1.0", Channel::Stable);
        let catalog = two.catalog();
        let mut install = Install {
            version: v("0.1.0"),
            channel: Channel::Stable,
            path: PathBuf::from("/engines/0.1.0"),
            kind: crate::InstallKind::Managed,
            installed: None,
            missing: false,
            releases: Some(two.fork.url()),
        };
        assert_eq!(catalog.of_install(&install).unwrap().source.name, "Fork");
        install.releases = None;
        assert_eq!(catalog.of_install(&install).unwrap().source.name, "Official", "older installs are official");
    }
}
