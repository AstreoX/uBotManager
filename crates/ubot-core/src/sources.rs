//! Loading index sources (SPEC §1.1): remote URLs with a cache, the local copy inside the installed
//! UBotCore, user files, and the embedded copy as the last resort.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::index::{self, IndexFile, SourceIndex};
use crate::model::{IndexSourceConfig, IndexSourceStatus};
use crate::paths;
use crate::settings::{LOCAL_SOURCE_ID, REMOTE_INDEX_URL, REMOTE_SOURCE_ID};
use crate::url::redact_credentials;

/// The copy of uBotCore's `Index/index.json` compiled into the manager.
pub const EMBEDDED_INDEX: &str = include_str!("../assets/index.json");
pub const EMBEDDED_SOURCE_ID: &str = "embedded";
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// An index larger than this is refused (downloaded, cached and local files alike): the real one
/// is a few dozen kilobytes, and the whole text is held in memory and parsed.
pub const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

pub fn embedded_index() -> IndexFile {
    index::parse(EMBEDDED_INDEX).expect("the embedded index is validated by tests")
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn time_rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Downloads index text. Abstracted for tests.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<String, String>;
}

/// HTTPS with a 10 s timeout. Proxies come from `HTTPS_PROXY` / `HTTP_PROXY`. Redirects are
/// followed only to https (or loopback) targets, and a body over [`MAX_INDEX_BYTES`] is refused.
pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn fetch(&self, url: &str) -> Result<String, String> {
        // The blocking client must not run inside an async runtime, so it gets its own thread.
        let url = url.to_string();
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .user_agent(concat!("uBotManager/", env!("CARGO_PKG_VERSION")))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= MAX_REDIRECTS {
                        attempt.error("too many redirects")
                    } else if !scheme_and_host_allowed(
                        attempt.url().scheme(),
                        attempt.url().host_str(),
                    ) {
                        attempt.error("redirect to a plain http:// URL refused")
                    } else {
                        attempt.follow()
                    }
                }))
                .build()
                .map_err(|error| error.to_string())?;
            let response = client
                .get(&url)
                .send()
                .map_err(|error| describe_reqwest_error(&error))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("HTTP {status}"));
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_INDEX_BYTES)
            {
                return Err(too_large());
            }
            let mut body = Vec::new();
            response
                .take(MAX_INDEX_BYTES + 1)
                .read_to_end(&mut body)
                .map_err(|error| format!("could not read the response: {error}"))?;
            if body.len() as u64 > MAX_INDEX_BYTES {
                return Err(too_large());
            }
            String::from_utf8(body).map_err(|_| "the index is not valid UTF-8".to_string())
        })
        .join()
        .unwrap_or_else(|_| Err("the download thread panicked".into()))
    }
}

fn too_large() -> String {
    format!(
        "the index is larger than {} MiB",
        MAX_INDEX_BYTES / 1024 / 1024
    )
}

fn describe_reqwest_error(error: &reqwest::Error) -> String {
    let text = if error.is_timeout() {
        "timed out".to_string()
    } else if error.is_connect() {
        "could not connect".to_string()
    } else if error.is_redirect() {
        format!(
            "a redirect was refused (at most {MAX_REDIRECTS}, and only to https URLs, are followed)"
        )
    } else {
        error.to_string()
    };
    redact_credentials(&text)
}

pub fn is_url(location: &str) -> bool {
    let lower = location.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

/// https anywhere; plain http only to the machine itself (local test servers).
pub(crate) fn scheme_and_host_allowed(scheme: &str, host: Option<&str>) -> bool {
    scheme.eq_ignore_ascii_case("https")
        || (scheme.eq_ignore_ascii_case("http") && host.is_some_and(is_loopback_host))
}

/// SPEC §1.1: URL sources are https. A plain `http://` URL is accepted only for the local machine.
pub fn check_index_url(location: &str) -> Result<(), String> {
    let location = location.trim();
    let Some((scheme, rest)) = location.split_once("://") else {
        return Err("not a URL".into());
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if scheme_and_host_allowed(scheme, Some(host)) {
        Ok(())
    } else {
        Err("plain http:// index sources are not allowed; use https://".into())
    }
}

/// What [`load_sources`] needs to know about the environment.
pub struct LoadContext<'a> {
    /// Re-fetch every remote source.
    pub sync: bool,
    /// Never fetch; caches only.
    pub offline: bool,
    /// Do not fetch in this scan because a sync follows: sources without a cache are skipped as
    /// "not fetched yet" instead of failing.
    pub defer_fetch: bool,
    pub cache_dir: &'a Path,
    pub project_dir: Option<&'a Path>,
    /// `<UBotCore>/Index/index.json` of the UBotCore installed in the open project.
    pub local_index: Option<PathBuf>,
    pub fetcher: &'a dyn Fetcher,
    /// Errors of fetches already tried in this session, so sources without a cache are not
    /// retried (with their timeout) on every refresh.
    pub failed_fetches: &'a mut HashMap<String, String>,
    /// Downloads made beforehand by [`prefetch`] (outside any lock), keyed by [`fetch_key`]. A
    /// source found here is not fetched again.
    pub prefetched: HashMap<String, Result<String, String>>,
}

/// A source that was tried and failed, for the `indexSourceFailed` problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFailure {
    pub id: String,
    pub name: String,
    pub location: String,
    pub error: String,
    pub from_cache: bool,
    /// A built-in source (the remote index or the local copy), as opposed to one the user added.
    pub builtin: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LoadedSources {
    pub statuses: Vec<IndexSourceStatus>,
    /// Successfully loaded indexes in merge order.
    pub indexes: Vec<SourceIndex>,
    pub failures: Vec<SourceFailure>,
}

/// The cache file of a source. The file name carries a hash of the location, so a source that
/// keeps its id but gets another URL (or a positional `--index` id such as `cli-1` that names a
/// different URL on the next run) never reads the previous URL's cache. The built-in remote source
/// keeps the plain name `index-ubot.json` of SPEC §1.1.
pub fn cache_file(cache_dir: &Path, id: &str, location: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let location = location.trim();
    if id == REMOTE_SOURCE_ID && location == REMOTE_INDEX_URL {
        return cache_dir.join(format!("index-{safe}.json"));
    }
    // FNV-1a over the location without credentials: stable across runs and releases.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in redact_credentials(location).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    cache_dir.join(format!("index-{safe}-{:08x}.json", hash >> 32))
}

/// Key of `failed_fetches` and `prefetched`: a failure belongs to a source *and* its location.
pub fn fetch_key(config: &IndexSourceConfig) -> String {
    format!(
        "{}\n{}",
        config.id,
        redact_credentials(config.location.trim())
    )
}

/// An I/O error in English; the operating system's own text follows the user's language, and the
/// CLI prints English.
fn describe_io_error(error: &std::io::Error) -> String {
    use std::io::ErrorKind;
    let text = match error.kind() {
        ErrorKind::NotFound => "file not found".to_string(),
        ErrorKind::PermissionDenied => "access denied".to_string(),
        ErrorKind::InvalidData => "the file is not valid UTF-8 text".to_string(),
        other => other.to_string(),
    };
    match error.raw_os_error() {
        Some(code) => format!("{text} (os error {code})"),
        None => text,
    }
}

fn read_index_file(path: &Path) -> Result<IndexFile, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| describe_io_error(&error))?
        .len();
    if size > MAX_INDEX_BYTES {
        return Err(too_large());
    }
    let (text, _) = paths::read_text(path).map_err(|error| describe_io_error(&error))?;
    index::parse(&text)
}

fn modified_time(path: &Path) -> Option<String> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .map(time_rfc3339)
}

pub fn load_sources(configs: &[IndexSourceConfig], context: &mut LoadContext<'_>) -> LoadedSources {
    let mut loaded = LoadedSources::default();
    for config in configs {
        let mut status = IndexSourceStatus {
            id: config.id.clone(),
            // A source without a name is called by its location (settings, `--index`), so the
            // name can hold credentials like the location does.
            name: redact_credentials(&config.name),
            location: redact_credentials(&config.location),
            builtin: config.builtin,
            enabled: config.enabled,
            ok: false,
            from_cache: false,
            error: None,
            packages: 0,
            fetched_at: None,
        };
        if !config.enabled {
            loaded.statuses.push(status);
            continue;
        }

        // The local copy is simply unavailable (not a failure) without an installed UBotCore that
        // ships Index/index.json.
        let mut unavailable = false;
        let result = if config.builtin && config.id == LOCAL_SOURCE_ID {
            match (&context.local_index, context.project_dir) {
                (Some(path), project_dir) => {
                    if let Some(project_dir) = project_dir {
                        status.location = paths::relative_display(path, project_dir);
                    }
                    if path.is_file() {
                        status.fetched_at = modified_time(path);
                        Some(read_index_file(path))
                    } else {
                        unavailable = true;
                        status.error =
                            Some("The installed UBotCore has no Index/index.json.".into());
                        None
                    }
                }
                (None, _) => {
                    unavailable = true;
                    status.error = Some("UBotCore is not installed in this project.".into());
                    None
                }
            }
        } else if is_url(&config.location) {
            let never_loaded = !cache_file(context.cache_dir, &config.id, &config.location)
                .is_file()
                && !context.failed_fetches.contains_key(&fetch_key(config));
            if let Err(error) = check_index_url(&config.location) {
                Some(Err(error))
            } else if context.defer_fetch && never_loaded {
                unavailable = true;
                status.error = Some("not fetched yet".into());
                None
            } else {
                Some(load_remote(config, context, &mut status))
            }
        } else {
            let location = PathBuf::from(config.location.trim());
            let path = match (location.is_absolute(), context.project_dir) {
                (true, _) => Some(location),
                (false, Some(project_dir)) => Some(paths::normalize(&project_dir.join(location))),
                (false, None) => None,
            };
            match path {
                Some(path) => {
                    status.fetched_at = modified_time(&path);
                    Some(read_index_file(&path))
                }
                None => Some(Err("a relative path needs an open project".into())),
            }
        };

        match result {
            Some(Ok(index)) => {
                status.packages = index.packages.len() as u32;
                status.ok = status.error.is_none();
                loaded.indexes.push(SourceIndex {
                    id: config.id.clone(),
                    index,
                });
            }
            Some(Err(error)) => {
                let error = redact_credentials(&error);
                status.ok = false;
                status.error = Some(error);
            }
            None => {}
        }
        if let Some(error) = &status.error {
            if !unavailable {
                loaded.failures.push(SourceFailure {
                    id: status.id.clone(),
                    name: status.name.clone(),
                    location: status.location.clone(),
                    error: error.clone(),
                    from_cache: status.from_cache,
                    builtin: config.builtin,
                });
            }
        }
        loaded.statuses.push(status);
    }

    if loaded.indexes.is_empty() {
        let embedded = embedded_index();
        loaded.statuses.push(IndexSourceStatus {
            id: EMBEDDED_SOURCE_ID.into(),
            name: "Built-in copy".into(),
            location: "embedded".into(),
            builtin: true,
            enabled: true,
            ok: true,
            from_cache: false,
            error: None,
            packages: embedded.packages.len() as u32,
            fetched_at: None,
        });
        loaded.indexes.push(SourceIndex {
            id: EMBEDDED_SOURCE_ID.into(),
            index: embedded,
        });
    }
    loaded
}

/// True when loading `config` now would download it: a sync was asked for, or the source was never
/// loaded and has not failed yet this session. Never when offline or deferred.
fn wants_fetch(
    config: &IndexSourceConfig,
    sync: bool,
    offline: bool,
    defer_fetch: bool,
    cache_dir: &Path,
    failed_fetches: &HashMap<String, String>,
) -> bool {
    if offline || defer_fetch {
        return false;
    }
    let has_cache = cache_file(cache_dir, &config.id, &config.location).is_file();
    sync || (!has_cache && !failed_fetches.contains_key(&fetch_key(config)))
}

/// Downloads, in parallel and without touching any shared state, the remote sources that loading
/// `configs` is going to fetch. The caller runs this *before* it takes the lock that serializes
/// rebuilds, so a slow or unreachable index server (10 s per source) never holds that lock; the
/// result goes to [`LoadContext::prefetched`].
pub fn prefetch(
    configs: &[IndexSourceConfig],
    sync: bool,
    offline: bool,
    defer_fetch: bool,
    cache_dir: &Path,
    failed_fetches: &HashMap<String, String>,
    fetcher: &dyn Fetcher,
) -> HashMap<String, Result<String, String>> {
    let wanted: Vec<&IndexSourceConfig> = configs
        .iter()
        .filter(|config| {
            config.enabled
                && !(config.builtin && config.id == LOCAL_SOURCE_ID)
                && is_url(&config.location)
                && check_index_url(&config.location).is_ok()
                && wants_fetch(
                    config,
                    sync,
                    offline,
                    defer_fetch,
                    cache_dir,
                    failed_fetches,
                )
        })
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = wanted
            .iter()
            .map(|config| {
                let location = config.location.trim().to_string();
                (
                    fetch_key(config),
                    scope.spawn(move || fetcher.fetch(&location)),
                )
            })
            .collect();
        handles
            .into_iter()
            .map(|(key, handle)| {
                let result = handle
                    .join()
                    .unwrap_or_else(|_| Err("the download thread panicked".into()));
                (key, result)
            })
            .collect()
    })
}

/// Fetches when syncing (or when there is no cache yet), falls back to the cache on failure.
/// A cache that was used after a failed fetch keeps the error in the status.
fn load_remote(
    config: &IndexSourceConfig,
    context: &mut LoadContext<'_>,
    status: &mut IndexSourceStatus,
) -> Result<IndexFile, String> {
    let cache = cache_file(context.cache_dir, &config.id, &config.location);
    let has_cache = cache.is_file();
    let key = fetch_key(config);
    let previous_failure = context.failed_fetches.get(&key).cloned();
    let should_fetch = wants_fetch(
        config,
        context.sync,
        context.offline,
        context.defer_fetch,
        context.cache_dir,
        context.failed_fetches,
    );

    let mut fetch_error = None;
    if should_fetch {
        let downloaded = match context.prefetched.remove(&key) {
            Some(result) => result,
            None => context.fetcher.fetch(config.location.trim()),
        };
        match downloaded
            .and_then(|text| {
                if text.len() as u64 > MAX_INDEX_BYTES {
                    Err(too_large())
                } else {
                    Ok(text)
                }
            })
            .and_then(|text| index::parse(&text).map(|index| (text, index)))
        {
            Ok((text, index)) => {
                context.failed_fetches.remove(&key);
                // Without a writable cache the source still works for this session.
                let _ = paths::write_atomic(&cache, text.as_bytes());
                status.fetched_at = Some(now_rfc3339());
                return Ok(index);
            }
            Err(error) => {
                let error = redact_credentials(&error);
                context.failed_fetches.insert(key, error.clone());
                fetch_error = Some(error);
            }
        }
    } else if !has_cache {
        fetch_error = Some(match (context.offline, previous_failure) {
            (_, Some(previous)) => previous,
            (true, None) => "offline and not cached".to_string(),
            (false, None) => "not fetched yet".to_string(),
        });
    }

    if has_cache {
        // "From cache" means the cache was used. A cache that cannot be read is no fallback: the
        // source fails, with both causes, and callers must not mistake it for a served copy.
        return match read_index_file(&cache) {
            Ok(index) => {
                status.from_cache = true;
                status.fetched_at = modified_time(&cache);
                status.error = fetch_error;
                Ok(index)
            }
            Err(cache_error) => Err(match fetch_error {
                Some(fetch_error) => format!("{fetch_error}; cached copy: {cache_error}"),
                None => format!("cached copy: {cache_error}"),
            }),
        };
    }
    Err(fetch_error.unwrap_or_else(|| "not available".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeFetcher {
        responses: Mutex<HashMap<String, Result<String, String>>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeFetcher {
        fn new(responses: &[(&str, Result<&str, &str>)]) -> Self {
            FakeFetcher {
                responses: Mutex::new(
                    responses
                        .iter()
                        .map(|(url, result)| {
                            (
                                url.to_string(),
                                result.map(str::to_string).map_err(str::to_string),
                            )
                        })
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn set(&self, url: &str, result: Result<&str, &str>) {
            self.responses.lock().unwrap().insert(
                url.into(),
                result.map(str::to_string).map_err(str::to_string),
            );
        }
        fn calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl Fetcher for FakeFetcher {
        fn fetch(&self, url: &str) -> Result<String, String> {
            self.calls.lock().unwrap().push(url.to_string());
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or(Err("HTTP 404 Not Found".into()))
        }
    }

    fn index_text(package: &str) -> String {
        format!(
            r#"{{ "formatVersion": 2, "name": "t", "packages": [ {{ "name": "{package}", "repository": "https://h/{package}.git",
                 "versions": [ {{ "version": "0.1.0", "ref": "v0.1.0" }} ] }} ] }}"#
        )
    }

    fn source(id: &str, location: &str, builtin: bool) -> IndexSourceConfig {
        IndexSourceConfig {
            id: id.into(),
            name: id.into(),
            location: location.into(),
            enabled: true,
            builtin,
        }
    }

    #[test]
    fn embedded_index_is_valid() {
        let validation = index::validate_text(EMBEDDED_INDEX);
        assert!(validation.errors.is_empty(), "{:#?}", validation.errors);
        assert!(embedded_index().find("UBotCore").is_some());
    }

    #[test]
    fn remote_cache_local_and_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let project_dir = dir.path().join("Project");
        let local = project_dir.join("Plugins/uBot/uBotCore/Index/index.json");
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(&local, index_text("Local")).unwrap();
        std::fs::write(project_dir.join("team.json"), index_text("Team")).unwrap();

        let remote_text = index_text("Remote");
        let fetcher = FakeFetcher::new(&[("https://h/index.json", Ok(remote_text.as_str()))]);
        let configs = vec![
            source("ubot", "https://user:secret@h/index.json", true),
            source("local", "<UBotCore>/Index/index.json", true),
            source("team", "team.json", false),
            IndexSourceConfig {
                enabled: false,
                ..source("off", "https://h/off.json", false)
            },
        ];
        let mut failed = HashMap::new();

        // The URL with credentials is never fetched as such by this fake: it fails, no cache yet.
        let mut context = LoadContext {
            sync: true,
            offline: false,
            defer_fetch: false,
            cache_dir: &cache_dir,
            project_dir: Some(&project_dir),
            local_index: Some(local.clone()),
            fetcher: &fetcher,
            failed_fetches: &mut failed,
            prefetched: HashMap::new(),
        };
        let loaded = load_sources(&configs, &mut context);
        assert_eq!(loaded.statuses.len(), 4);
        assert_eq!(loaded.statuses[0].location, "https://***@h/index.json");
        assert!(!loaded.statuses[0].ok);
        assert_eq!(
            loaded.statuses[0].error.as_deref(),
            Some("HTTP 404 Not Found")
        );
        assert_eq!(
            loaded.statuses[1].location,
            "Plugins/uBot/uBotCore/Index/index.json"
        );
        assert!(loaded.statuses[1].ok && loaded.statuses[2].ok);
        assert!(!loaded.statuses[3].ok && loaded.statuses[3].error.is_none());
        assert_eq!(
            loaded
                .indexes
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["local", "team"]
        );
        assert_eq!(loaded.failures.len(), 1);
        assert!(!loaded.failures[0].from_cache);

        // A successful sync writes the cache.
        let configs = vec![source("ubot", "https://h/index.json", true)];
        let mut context = LoadContext {
            sync: true,
            offline: false,
            defer_fetch: false,
            cache_dir: &cache_dir,
            project_dir: None,
            local_index: None,
            fetcher: &fetcher,
            failed_fetches: &mut failed,
            prefetched: HashMap::new(),
        };
        let loaded = load_sources(&configs, &mut context);
        assert!(loaded.statuses[0].ok && !loaded.statuses[0].from_cache);
        assert!(loaded.statuses[0].fetched_at.is_some());
        assert!(cache_file(&cache_dir, "ubot", "https://h/index.json").is_file());

        // Not syncing: the cache is used without fetching.
        let calls = fetcher.calls();
        context.sync = false;
        let loaded = load_sources(&configs, &mut context);
        assert_eq!(fetcher.calls(), calls);
        assert!(loaded.statuses[0].ok && loaded.statuses[0].from_cache);
        assert_eq!(loaded.indexes[0].index.packages[0].name, "Remote");

        // A failed sync falls back to the cache and keeps the error.
        fetcher.set("https://h/index.json", Err("timed out"));
        context.sync = true;
        let loaded = load_sources(&configs, &mut context);
        let status = &loaded.statuses[0];
        assert!(!status.ok && status.from_cache);
        assert_eq!(status.error.as_deref(), Some("timed out"));
        assert_eq!(loaded.indexes.len(), 1);
        assert!(loaded.failures[0].from_cache);

        // Offline without a cache: the embedded copy is used.
        let configs = vec![
            source("other", "https://h/other.json", false),
            source("local", "x", true),
        ];
        context.offline = true;
        let loaded = load_sources(&configs, &mut context);
        assert_eq!(
            loaded.statuses[0].error.as_deref(),
            Some("offline and not cached")
        );
        assert_eq!(
            loaded.statuses[1].error.as_deref(),
            Some("UBotCore is not installed in this project.")
        );
        assert_eq!(
            loaded.failures.len(),
            1,
            "an unavailable local copy is not a failure"
        );

        // An installed UBotCore without Index/index.json is not a failure either.
        context.local_index = Some(dir.path().join("NoIndex").join("index.json"));
        let loaded = load_sources(&configs[1..], &mut context);
        assert!(loaded.failures.is_empty());
        assert_eq!(
            loaded.statuses[0].error.as_deref(),
            Some("The installed UBotCore has no Index/index.json.")
        );
        context.local_index = None;
        assert_eq!(loaded.statuses.last().unwrap().id, EMBEDDED_SOURCE_ID);
        assert_eq!(loaded.indexes[0].id, EMBEDDED_SOURCE_ID);

        // A source that failed once is not fetched again until a sync.
        context.offline = false;
        context.sync = false;
        let calls = fetcher.calls();
        load_sources(&configs[..1], &mut context);
        let after_first = fetcher.calls();
        assert_eq!(after_first, calls + 1);
        let loaded = load_sources(&configs[..1], &mut context);
        assert_eq!(fetcher.calls(), after_first);
        assert_eq!(
            loaded.statuses[0].error.as_deref(),
            Some("HTTP 404 Not Found")
        );
    }

    #[test]
    fn a_deferred_scan_does_not_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let remote_text = index_text("Remote");
        let fetcher = FakeFetcher::new(&[("https://h/index.json", Ok(remote_text.as_str()))]);
        let configs = vec![source("ubot", "https://h/index.json", true)];
        let mut failed = HashMap::new();
        let mut context = LoadContext {
            sync: false,
            offline: false,
            defer_fetch: true,
            cache_dir: &cache_dir,
            project_dir: None,
            local_index: None,
            fetcher: &fetcher,
            failed_fetches: &mut failed,
            prefetched: HashMap::new(),
        };

        // Never loaded: skipped without a failure; the embedded copy fills in.
        let loaded = load_sources(&configs, &mut context);
        assert_eq!(fetcher.calls(), 0);
        assert_eq!(loaded.statuses[0].error.as_deref(), Some("not fetched yet"));
        assert!(loaded.failures.is_empty());
        assert_eq!(loaded.indexes[0].id, EMBEDDED_SOURCE_ID);

        // Cached: the cache is used, still without fetching.
        context.defer_fetch = false;
        context.sync = true;
        load_sources(&configs, &mut context);
        assert_eq!(fetcher.calls(), 1);
        context.defer_fetch = true;
        let loaded = load_sources(&configs, &mut context);
        assert_eq!(fetcher.calls(), 1);
        assert!(loaded.statuses[0].ok && loaded.statuses[0].from_cache);
        assert_eq!(loaded.indexes[0].index.packages[0].name, "Remote");
    }

    fn context<'a>(
        cache_dir: &'a Path,
        fetcher: &'a FakeFetcher,
        failed: &'a mut HashMap<String, String>,
    ) -> LoadContext<'a> {
        LoadContext {
            sync: true,
            offline: false,
            defer_fetch: false,
            cache_dir,
            project_dir: None,
            local_index: None,
            fetcher,
            failed_fetches: failed,
            prefetched: HashMap::new(),
        }
    }

    #[test]
    fn the_cache_belongs_to_the_location_not_just_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let a_text = index_text("AAA");
        let fetcher = FakeFetcher::new(&[("https://h/a.json", Ok(a_text.as_str()))]);
        let mut failed = HashMap::new();
        let mut load = context(&cache_dir, &fetcher, &mut failed);

        // `--index` ids are positional: cli-1 names a.json now and b.json on the next run.
        let loaded = load_sources(&[source("cli-1", "https://h/a.json", false)], &mut load);
        assert!(loaded.statuses[0].ok);

        load.sync = false;
        load.offline = true;
        let loaded = load_sources(&[source("cli-1", "https://h/b.json", false)], &mut load);
        let status = &loaded.statuses[0];
        assert!(!status.ok && !status.from_cache, "{status:?}");
        assert_eq!(status.error.as_deref(), Some("offline and not cached"));
        assert_eq!(loaded.indexes[0].id, EMBEDDED_SOURCE_ID);

        // The cache of a.json still serves a.json.
        let loaded = load_sources(&[source("cli-1", "https://h/a.json", false)], &mut load);
        assert!(loaded.statuses[0].from_cache);
        assert_eq!(loaded.indexes[0].index.packages[0].name, "AAA");

        // A failure of b.json does not stop the first fetch of an edited location c.json.
        load.offline = false;
        load.sync = false;
        let b = load_sources(&[source("cli-1", "https://h/b.json", false)], &mut load);
        assert_eq!(b.statuses[0].error.as_deref(), Some("HTTP 404 Not Found"));
        let calls = fetcher.calls();
        load_sources(&[source("cli-1", "https://h/c.json", false)], &mut load);
        assert_eq!(fetcher.calls(), calls + 1);

        // Credentials do not change the cache identity; the built-in name stays plain.
        assert_eq!(
            cache_file(&cache_dir, "x", "https://u:p@h/a.json"),
            cache_file(&cache_dir, "x", "https://u:q@h/a.json")
        );
        assert_eq!(
            cache_file(&cache_dir, REMOTE_SOURCE_ID, REMOTE_INDEX_URL),
            cache_dir.join("index-ubot.json")
        );
    }

    #[test]
    fn a_cache_that_cannot_be_read_is_not_a_served_copy() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let url = "https://127.0.0.1:1/i.json";
        std::fs::write(cache_file(&cache_dir, "cli-1", url), "{ this is not json").unwrap();
        let fetcher = FakeFetcher::new(&[(url, Err("timed out"))]);
        let mut failed = HashMap::new();
        let mut load = context(&cache_dir, &fetcher, &mut failed);

        // The fetch failed and so does the cache: both causes are reported, and the source is
        // not "from cache" (callers stop on a failed source that served nothing).
        let loaded = load_sources(&[source("cli-1", url, false)], &mut load);
        let status = &loaded.statuses[0];
        assert!(!status.ok && !status.from_cache, "{status:?}");
        let error = status.error.as_deref().unwrap();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("cached copy: Invalid JSON"), "{error}");
        assert!(!loaded.failures[0].from_cache);
        assert_eq!(loaded.indexes[0].id, EMBEDDED_SOURCE_ID);

        // Offline there is no fetch error, only the cache's.
        load.sync = false;
        load.offline = true;
        let loaded = load_sources(&[source("cli-1", url, false)], &mut load);
        let status = &loaded.statuses[0];
        assert!(!status.ok && !status.from_cache, "{status:?}");
        let error = status.error.as_deref().unwrap();
        assert!(
            error.starts_with("cached copy: Invalid JSON") && !error.contains("timed out"),
            "{error}"
        );
        assert!(!loaded.failures[0].from_cache);

        // A cache that parses still serves, with the fetch error kept.
        std::fs::write(
            cache_file(&cache_dir, "cli-1", url),
            index_text("Cached").as_bytes(),
        )
        .unwrap();
        load.sync = true;
        load.offline = false;
        let loaded = load_sources(&[source("cli-1", url, false)], &mut load);
        let status = &loaded.statuses[0];
        assert!(!status.ok && status.from_cache, "{status:?}");
        assert_eq!(status.error.as_deref(), Some("timed out"));
        assert!(loaded.failures[0].from_cache);
        assert_eq!(loaded.indexes[0].index.packages[0].name, "Cached");
    }

    #[test]
    fn credentials_in_a_source_name_are_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let url = "https://user:SECRET@127.0.0.1:1/i.json";
        let fetcher = FakeFetcher::new(&[(url, Err("could not connect"))]);
        let mut failed = HashMap::new();
        let mut load = context(&cache_dir, &fetcher, &mut failed);

        // `--index` sources and sources saved without a name are called by their location.
        let unnamed = IndexSourceConfig {
            name: url.into(),
            ..source("cli-1", url, false)
        };
        let named = IndexSourceConfig {
            name: "Team index".into(),
            ..source("team", "https://127.0.0.1:1/team.json", false)
        };
        let loaded = load_sources(&[unnamed, named], &mut load);
        assert_eq!(loaded.statuses[0].name, "https://***@127.0.0.1:1/i.json");
        assert_eq!(
            loaded.statuses[0].location,
            "https://***@127.0.0.1:1/i.json"
        );
        assert_eq!(loaded.failures[0].name, "https://***@127.0.0.1:1/i.json");
        assert_eq!(loaded.statuses[1].name, "Team index", "plain names stay");
        for status in &loaded.statuses {
            assert!(!format!("{status:?}").contains("SECRET"), "{status:?}");
        }
        for failure in &loaded.failures {
            assert!(!format!("{failure:?}").contains("SECRET"), "{failure:?}");
        }
    }

    #[test]
    fn plain_http_sources_are_refused_except_for_the_local_machine() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let text = index_text("Local");
        let fetcher = FakeFetcher::new(&[
            ("http://example.com/i.json", Ok(text.as_str())),
            ("http://127.0.0.1:18765/i.json", Ok(text.as_str())),
        ]);
        let mut failed = HashMap::new();
        let mut load = context(&cache_dir, &fetcher, &mut failed);

        let loaded = load_sources(
            &[source("t", "http://example.com/i.json", false)],
            &mut load,
        );
        assert!(!loaded.statuses[0].ok);
        assert!(loaded.statuses[0]
            .error
            .as_deref()
            .unwrap()
            .contains("https"));
        assert_eq!(fetcher.calls(), 0, "never contacted");

        let loaded = load_sources(
            &[source("t", "http://127.0.0.1:18765/i.json", false)],
            &mut load,
        );
        assert!(loaded.statuses[0].ok);

        assert!(check_index_url("https://a/b").is_ok());
        assert!(check_index_url("HTTPS://a/b").is_ok());
        assert!(check_index_url("http://localhost:8080/x").is_ok());
        assert!(check_index_url("http://[::1]:80/x").is_ok());
        assert!(check_index_url("http://user@127.0.0.1/x").is_ok());
        assert!(check_index_url("http://a.localhost/x").is_ok());
        assert!(check_index_url("http://evil.com/localhost").is_err());
        assert!(check_index_url("http://localhost.evil.com/x").is_err());
        assert!(check_index_url("http://127.0.0.1@evil.com/x").is_err());
    }

    #[test]
    fn an_oversized_index_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_INDEX_BYTES + 1).unwrap();
        drop(file);
        let error = read_index_file(&path).unwrap_err();
        assert!(error.contains("larger than 8 MiB"), "{error}");

        // The same limit applies to a download by any fetcher.
        let cache_dir = dir.path().join("cache");
        let big = " ".repeat(MAX_INDEX_BYTES as usize + 1);
        let fetcher = FakeFetcher::new(&[("https://h/i.json", Ok(big.as_str()))]);
        let mut failed = HashMap::new();
        let mut load = context(&cache_dir, &fetcher, &mut failed);
        let loaded = load_sources(&[source("t", "https://h/i.json", false)], &mut load);
        assert!(loaded.statuses[0]
            .error
            .as_deref()
            .unwrap()
            .contains("larger than 8 MiB"));
        assert!(!cache_file(&cache_dir, "t", "https://h/i.json").exists());
    }

    #[test]
    fn prefetched_downloads_are_not_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let text = index_text("Remote");
        let fetcher = FakeFetcher::new(&[("https://h/index.json", Ok(text.as_str()))]);
        let configs = vec![
            source("ubot", "https://h/index.json", true),
            source("local", "<UBotCore>/Index/index.json", true),
            IndexSourceConfig {
                enabled: false,
                ..source("off", "https://h/off.json", false)
            },
            source("team", "http://example.com/x.json", false),
        ];
        let mut failed = HashMap::new();

        let nothing = |sync, offline, defer| {
            prefetch(
                &configs,
                sync,
                offline,
                defer,
                &cache_dir,
                &HashMap::new(),
                &fetcher,
            )
        };
        assert!(nothing(true, true, false).is_empty(), "offline");
        assert!(nothing(true, false, true).is_empty(), "deferred");
        assert_eq!(fetcher.calls(), 0);

        let prefetched = nothing(true, false, false);
        assert_eq!(
            fetcher.calls(),
            1,
            "only the enabled https source that is not the local copy"
        );
        assert_eq!(prefetched.len(), 1);

        let mut load = context(&cache_dir, &fetcher, &mut failed);
        load.prefetched = prefetched;
        let loaded = load_sources(&configs, &mut load);
        assert_eq!(fetcher.calls(), 1, "the prefetched download is used");
        assert!(loaded.statuses[0].ok && !loaded.statuses[0].from_cache);
        assert!(loaded.statuses[0].fetched_at.is_some());
        assert!(cache_file(&cache_dir, "ubot", "https://h/index.json").is_file());

        // Without sync a cached source is not downloaded; one that was never loaded is.
        assert!(prefetch(
            &configs[..1],
            false,
            false,
            false,
            &cache_dir,
            &HashMap::new(),
            &fetcher
        )
        .is_empty());
        let never_loaded = vec![source("fresh", "https://h/index.json", false)];
        assert_eq!(
            prefetch(
                &never_loaded,
                false,
                false,
                false,
                &cache_dir,
                &HashMap::new(),
                &fetcher
            )
            .len(),
            1
        );
    }

    /// Serves one HTTP connection on the loopback address and returns the URL.
    fn serve_once(respond: impl FnOnce(&mut std::net::TcpStream) + Send + 'static) -> String {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => return,
                    }
                }
                respond(&mut stream);
            }
        });
        format!("http://127.0.0.1:{port}/index.json")
    }

    #[test]
    fn the_http_fetcher_limits_size_and_redirects() {
        use std::io::Write;
        // The machine's proxy settings must not catch the loopback requests.
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");

        let text = index_text("Served");
        let body = text.clone();
        let url = serve_once(move |stream| {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        assert_eq!(HttpFetcher.fetch(&url).unwrap(), text);

        // A declared size over the limit is refused without reading the body.
        let url = serve_once(|stream| {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_INDEX_BYTES + 1
            );
        });
        let error = HttpFetcher.fetch(&url).unwrap_err();
        assert!(error.contains("larger than 8 MiB"), "{error}");

        // So is a body that grows past it without declaring a size.
        let url = serve_once(|stream| {
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let chunk = vec![b' '; 64 * 1024];
            for _ in 0..(MAX_INDEX_BYTES as usize / chunk.len() + 8) {
                if stream.write_all(&chunk).is_err() {
                    break;
                }
            }
        });
        let error = HttpFetcher.fetch(&url).unwrap_err();
        assert!(error.contains("larger than 8 MiB"), "{error}");

        // An https index is not redirected to plain http on another host.
        let url = serve_once(|stream| {
            let _ = write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://example.com/index.json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        });
        let error = HttpFetcher.fetch(&url).unwrap_err();
        assert!(error.contains("redirect was refused"), "{error}");

        // Failures stay readable.
        let error = HttpFetcher
            .fetch("http://127.0.0.1:1/index.json")
            .unwrap_err();
        assert_eq!(error, "could not connect");
    }
}
