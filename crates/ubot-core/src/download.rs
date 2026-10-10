//! Fetching prebuilt archives (SPEC §5.3, `download` steps): an HTTPS download, or a copy of a
//! `file://` URL or local path, streamed to a file with its SHA-256 computed on the way, a size
//! limit, progress reports and cancelation.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::index::MAX_BINARY_BYTES;
use crate::prebuilt::{hex, Stop};
use crate::sources::scheme_and_host_allowed;
use crate::url::{is_absolute_local_path, redact_credentials};

/// Connecting, and every read, give up after this long.
pub const TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_REDIRECTS: usize = 5;
const CHUNK: usize = 64 * 1024;
const POLL: Duration = Duration::from_millis(100);

/// A finished transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub bytes: u64,
    /// Lower-case hex.
    pub sha256: String,
}

/// The file a `file://` URL or an absolute local path names; `None` for anything else.
pub fn local_path(url: &str) -> Option<PathBuf> {
    let url = url.trim();
    if url
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file://"))
    {
        return reqwest::Url::parse(url).ok()?.to_file_path().ok();
    }
    is_absolute_local_path(url).then(|| PathBuf::from(url))
}

fn describe(error: &reqwest::Error) -> String {
    let text = if error.is_timeout() {
        "timed out".to_string()
    } else if error.is_connect() {
        "could not connect".to_string()
    } else if error.is_redirect() {
        format!(
            "a redirect was refused (at most {MAX_REDIRECTS}, never from https to http, and http only on this machine)"
        )
    } else {
        // reqwest names the URL; only the host and file may be shown (SPEC §5.3).
        let mut error_text = error.to_string();
        if let Some(url) = error.url() {
            error_text = error_text.replace(
                url.as_str(),
                &crate::url::display_download_source(url.as_str()),
            );
        }
        error_text
    };
    redact_credentials(&text)
}

/// Opens the source: a local file, or an HTTP response (https, or http on this machine for test
/// servers). Returns the reader and the length the source declares.
fn open(url: &str) -> Result<(Box<dyn Read + Send>, Option<u64>), String> {
    if let Some(path) = local_path(url) {
        let file = fs::File::open(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let length = file.metadata().ok().map(|metadata| metadata.len());
        return Ok((Box::new(file), length));
    }
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "not a URL".to_string())?;
    if !scheme_and_host_allowed(parsed.scheme(), parsed.host_str()) {
        return Err(
            "only https:// URLs are downloaded (plain http only from this machine)".to_string(),
        );
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(TIMEOUT)
        .timeout(TIMEOUT)
        .user_agent(concat!("uBotManager/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let from_https = attempt
                .previous()
                .last()
                .is_some_and(|previous| previous.scheme().eq_ignore_ascii_case("https"));
            let to = attempt.url();
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if from_https && !to.scheme().eq_ignore_ascii_case("https") {
                attempt.error("redirect from https to http refused")
            } else if !scheme_and_host_allowed(to.scheme(), to.host_str()) {
                attempt.error("redirect to a plain http:// URL refused")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(parsed)
        .send()
        .map_err(|error| describe(&error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let length = response.content_length();
    Ok((Box::new(response), length))
}

enum Event {
    Opened(Option<u64>),
    Data(Vec<u8>),
    End,
    Error(String),
}

/// Fetches `url` into `dest` (created or replaced). `expected_size` (the index's `size`) is
/// enforced exactly; without it the limit is [`MAX_BINARY_BYTES`]. `progress` gets the bytes so
/// far and the total when known. The transfer runs on its own thread, so a cancel is seen within
/// a tenth of a second even while the network is silent; the caller removes `dest` after a
/// failure or cancel.
pub fn fetch(
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Fetched, Stop> {
    let limit = expected_size
        .unwrap_or(MAX_BINARY_BYTES)
        .min(MAX_BINARY_BYTES);
    let too_large = || match expected_size {
        Some(size) => format!("the download is larger than the {size} bytes the index lists"),
        None => format!(
            "the download is larger than {} GiB",
            MAX_BINARY_BYTES / 1024 / 1024 / 1024
        ),
    };

    let (sender, receiver) = mpsc::sync_channel::<Event>(16);
    let source = url.to_string();
    thread::Builder::new()
        .name("ubot-download".into())
        .spawn(move || {
            let (mut reader, length) = match open(&source) {
                Ok(opened) => opened,
                Err(error) => {
                    let _ = sender.send(Event::Error(error));
                    return;
                }
            };
            if sender.send(Event::Opened(length)).is_err() {
                return;
            }
            let mut buffer = vec![0u8; CHUNK];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(Event::End);
                        return;
                    }
                    Ok(read) => {
                        // The receiver is gone after a cancel or a failure: stop.
                        if sender.send(Event::Data(buffer[..read].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let text = if error.kind() == std::io::ErrorKind::TimedOut
                            || error.to_string().contains("timed out")
                        {
                            "timed out".to_string()
                        } else {
                            redact_credentials(&error.to_string())
                        };
                        let _ =
                            sender.send(Event::Error(format!("the transfer broke off: {text}")));
                        return;
                    }
                }
            }
        })
        .map_err(|error| Stop::Failed(format!("cannot start the download: {error}")))?;

    let mut file = fs::File::create(dest)
        .map_err(|error| Stop::Failed(format!("cannot create {}: {error}", dest.display())))?;
    let write_error =
        |error: std::io::Error| Stop::Failed(format!("cannot write {}: {error}", dest.display()));
    let mut hasher = Sha256::new();
    let mut done: u64 = 0;
    let mut total = expected_size;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Stop::Canceled);
        }
        match receiver.recv_timeout(POLL) {
            Ok(Event::Opened(length)) => {
                if let Some(length) = length {
                    match expected_size {
                        Some(size) if length != size => {
                            return Err(Stop::Failed(format!(
                                "the download has {length} bytes, the index lists {size}"
                            )))
                        }
                        _ if length > limit => return Err(Stop::Failed(too_large())),
                        _ => {}
                    }
                }
                total = expected_size.or(length);
                progress(0, total);
            }
            Ok(Event::Data(chunk)) => {
                done += chunk.len() as u64;
                if done > limit {
                    return Err(Stop::Failed(too_large()));
                }
                hasher.update(&chunk);
                file.write_all(&chunk).map_err(write_error)?;
                progress(done, total);
            }
            Ok(Event::End) => break,
            Ok(Event::Error(error)) => return Err(Stop::Failed(error)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Stop::failed("the download stopped unexpectedly"))
            }
        }
    }
    file.sync_all().map_err(write_error)?;
    drop(file);
    if let Some(size) = expected_size {
        if done != size {
            return Err(Stop::Failed(format!(
                "the download has {done} bytes, the index lists {size}"
            )));
        }
    }
    Ok(Fetched {
        bytes: done,
        sha256: hex(&hasher.finalize()),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    /// Serves HTTP on 127.0.0.1: `respond` gets every connection's request path and writes the
    /// answer. Returns the base URL `http://127.0.0.1:<port>`.
    pub fn serve(respond: impl Fn(&str, &mut TcpStream) + Send + Sync + 'static) -> String {
        // The machine's proxy settings must not catch the loopback requests.
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let respond = std::sync::Arc::new(respond);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let respond = respond.clone();
                thread::spawn(move || {
                    let mut request = Vec::new();
                    let mut byte = [0u8; 1];
                    while !request.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte) {
                            Ok(1) => request.push(byte[0]),
                            _ => return,
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                    respond(&path, &mut stream);
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    pub fn ok_body(stream: &mut TcpStream, body: &[u8]) {
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// The folder of the download, the result and the progress reports.
    type Run = (
        tempfile::TempDir,
        Result<Fetched, Stop>,
        Vec<(u64, Option<u64>)>,
    );

    fn run(url: &str, size: Option<u64>) -> Run {
        let dir = tempfile::tempdir().unwrap();
        let mut reports = Vec::new();
        let result = fetch(
            url,
            &dir.path().join("out.part"),
            size,
            &AtomicBool::new(false),
            &mut |done, total| reports.push((done, total)),
        );
        (dir, result, reports)
    }

    #[test]
    fn copies_local_files_and_file_urls() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.zip");
        let body = vec![7u8; 200_000];
        fs::write(&source, &body).unwrap();
        for url in [
            source.to_string_lossy().into_owned(),
            reqwest::Url::from_file_path(&source).unwrap().to_string(),
        ] {
            let (out, result, reports) = run(&url, Some(body.len() as u64));
            let fetched = result.unwrap();
            assert_eq!(fetched.bytes, 200_000);
            assert_eq!(fetched.sha256, sha(&body));
            assert_eq!(fs::read(out.path().join("out.part")).unwrap(), body);
            assert_eq!(reports.first(), Some(&(0, Some(200_000))));
            assert_eq!(reports.last(), Some(&(200_000, Some(200_000))));
        }
        let (_out, result, _) = run(&dir.path().join("missing.zip").to_string_lossy(), None);
        assert!(matches!(result, Err(Stop::Failed(error)) if error.contains("cannot read")));
        assert_eq!(local_path("https://h/x.zip"), None);
        assert_eq!(local_path("relative/x.zip"), None);
    }

    #[test]
    fn downloads_over_http_with_redirects_and_limits() {
        let body: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
        let served = body.clone();
        let base = serve(move |path, stream| match path {
            "/x.zip" => ok_body(stream, &served),
            "/redirect" => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: /x.zip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
            "/to-plain-http" => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: http://example.com/x.zip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
            "/loop" => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
            "/unsized" => {
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(&[1u8; 5000]);
            }
            _ => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });

        let (_dir, result, reports) = run(&format!("{base}/x.zip"), None);
        let fetched = result.unwrap();
        assert_eq!(fetched.sha256, sha(&body));
        assert_eq!(reports.last(), Some(&(300_000, Some(300_000))));

        let (_dir, result, _) = run(&format!("{base}/redirect"), Some(300_000));
        assert_eq!(result.unwrap().sha256, sha(&body));

        // A size that the server does not match.
        let (_dir, result, _) = run(&format!("{base}/x.zip"), Some(1000));
        assert!(
            matches!(&result, Err(Stop::Failed(error)) if error.contains("the index lists 1000")),
            "{result:?}"
        );
        let (_dir, result, _) = run(&format!("{base}/unsized"), Some(1000));
        assert!(
            matches!(&result, Err(Stop::Failed(error)) if error.contains("larger than the 1000 bytes")),
            "{result:?}"
        );
        let (_dir, result, reports) = run(&format!("{base}/unsized"), None);
        assert_eq!(result.unwrap().bytes, 5000);
        assert_eq!(reports.last(), Some(&(5000, None)));

        for (path, fragment) in [
            ("/to-plain-http", "redirect was refused"),
            ("/loop", "redirect was refused"),
            ("/missing", "HTTP 404"),
        ] {
            let (_dir, result, _) = run(&format!("{base}{path}"), None);
            assert!(
                matches!(&result, Err(Stop::Failed(error)) if error.contains(fragment)),
                "{path}: {result:?}"
            );
        }
        let (_dir, result, _) = run("http://example.com/x.zip", None);
        assert!(
            matches!(&result, Err(Stop::Failed(error)) if error.contains("only https://")),
            "{result:?}"
        );
    }

    #[test]
    fn a_cancel_stops_a_silent_download() {
        let base = serve(|_, stream| {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nab"
            );
            thread::sleep(Duration::from_secs(5));
        });
        let dir = tempfile::tempdir().unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            flag.store(true, Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let result = fetch(
            &format!("{base}/x.zip"),
            &dir.path().join("out.part"),
            None,
            &cancel,
            &mut |_, _| {},
        );
        assert_eq!(result, Err(Stop::Canceled));
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
