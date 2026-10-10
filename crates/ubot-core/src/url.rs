//! Repository URL policy (SPEC §5.4), credential redaction and install folder names.

/// Outcome of checking a repository URL against the policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlVerdict {
    Allowed,
    /// Allowed, but worth a warning (plain `http://`).
    Insecure,
    Rejected(String),
}

impl UrlVerdict {
    pub fn is_allowed(&self) -> bool {
        !matches!(self, UrlVerdict::Rejected(_))
    }
}

/// Allowed: `https://…`, `http://…` (insecure), `ssh://…`, scp-like `user@host:path`, `file://…`
/// and absolute local paths. Rejected: anything starting with '-', remote helper transports
/// (`<transport>::<address>` such as `ext::` and `fd::`), control characters and whitespace. A `::`
/// elsewhere is fine: it is part of an IPv6 address (`https://[::1]/team/uBotLocal.git`).
pub fn check_repository_url(url: &str) -> UrlVerdict {
    if url.is_empty() {
        return UrlVerdict::Rejected("the repository URL is empty".into());
    }
    if url.starts_with('-') || url.contains("://-") {
        return UrlVerdict::Rejected("the repository URL must not start with '-'".into());
    }
    if url
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '"')
    {
        return UrlVerdict::Rejected(
            "the repository URL contains whitespace, quotes or control characters".into(),
        );
    }
    // Same rule as FUBotPackageService::IsRepositoryUrlAllowed: the text before the first `::`
    // names a remote helper only when it is a transport name; `[::1]` and `git@[::1]:x` do not.
    if let Some(helper) = url.find("::") {
        if helper > 0
            && url[..helper]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        {
            return UrlVerdict::Rejected(
                "remote helper URLs (<transport>::<address>) are not allowed".into(),
            );
        }
    }

    if let Some(scheme_end) = url.find("://") {
        let scheme = url[..scheme_end].to_ascii_lowercase();
        let rest = &url[scheme_end + 3..];
        return match scheme.as_str() {
            "https" | "ssh" if !rest.is_empty() => UrlVerdict::Allowed,
            "http" if !rest.is_empty() => UrlVerdict::Insecure,
            "file" if !rest.is_empty() => UrlVerdict::Allowed,
            "https" | "http" | "ssh" | "file" => {
                UrlVerdict::Rejected("the repository URL has no address".into())
            }
            _ => UrlVerdict::Rejected(format!(
                "the URL scheme '{scheme}' is not allowed (use https, ssh or file)"
            )),
        };
    }

    if is_absolute_local_path(url) {
        return UrlVerdict::Allowed;
    }
    if is_scp_like(url) {
        return UrlVerdict::Allowed;
    }
    UrlVerdict::Rejected(
        "use an https://, ssh:// or file:// URL, user@host:path or an absolute path".into(),
    )
}

/// The URL policy of prebuilt archives (`binaries[engine][platform].url`, SPEC §1): the
/// repository policy, and then only `https://`, `file://` or an absolute local path (team mirrors,
/// tests). Plain `http://`, `ssh://` and `user@host:path` are refused, and so is a backslash in an
/// `https://` URL: the downloader's parser ends the host at a `\` where a reader may not, so the
/// host a person sees would not have to be the host that is contacted.
pub fn check_binary_url(url: &str) -> Result<(), String> {
    if let UrlVerdict::Rejected(reason) = check_repository_url(url) {
        return Err(reason);
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        if url.contains('\\') {
            return Err("backslashes are not allowed in https:// binary URLs".into());
        }
        Ok(())
    } else if lower.starts_with("file://") || is_absolute_local_path(url) {
        Ok(())
    } else if lower.starts_with("http://") {
        Err("plain http:// is not allowed for binaries; use https://".into())
    } else {
        Err("use an https:// or file:// URL or an absolute path".into())
    }
}

/// What a step detail shows of an archive's location: the host and the file name
/// (`github.com/…/UBotSensor-0.1.0-UE5.5-Win64.zip`), never the whole URL with its query or
/// credentials. `file://` URLs and local paths are shown as their path. The host of a web URL is
/// the one the downloader's parser (reqwest's) finds, so what is shown is what is contacted.
pub fn display_download_source(url: &str) -> String {
    let url = url.trim();
    let without_query = url.split(['?', '#']).next().unwrap_or_default();
    for scheme in ["https://", "http://"] {
        if strip_prefix_ignore_case(without_query, scheme).is_some() {
            let Ok(parsed) = reqwest::Url::parse(without_query) else {
                return redact_credentials(without_query);
            };
            let host = match (parsed.host_str(), parsed.port()) {
                (Some(host), Some(port)) => format!("{host}:{port}"),
                (Some(host), None) => host.to_string(),
                (None, _) => return redact_credentials(without_query),
            };
            let segments: Vec<&str> = parsed
                .path_segments()
                .map(|segments| segments.filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            return match segments.as_slice() {
                [] => host,
                [file] => format!("{host}/{file}"),
                [.., file] => format!("{host}/…/{file}"),
            };
        }
    }
    if let Some(rest) = strip_prefix_ignore_case(without_query, "file://") {
        // file:///C:/x.zip -> C:/x.zip; file://server/share/x.zip -> //server/share/x.zip
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            return rest[1..].to_string();
        }
        if rest.starts_with('/') {
            return rest.to_string();
        }
        return format!("//{rest}");
    }
    redact_credentials(url)
}

/// A URL as the manager stores it in files that stay in the project (the install marker): web
/// and `file://` URLs lose their credentials, query and fragment, which is where signed download
/// links keep their tokens (the SHA-256 identifies the archive anyway). An absolute local path is
/// kept as written, because `#` and `?` are part of a Windows file name.
pub fn strip_url_secrets(url: &str) -> String {
    let url = url.trim();
    let redacted = redact_credentials(url);
    let has_scheme = ["https://", "http://", "file://"]
        .iter()
        .any(|scheme| strip_prefix_ignore_case(&redacted, scheme).is_some());
    if has_scheme {
        redacted
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .to_string()
    } else {
        redacted
    }
}

/// `C:\…`, `C:/…`, `\\server\share\…` or `/…`.
pub fn is_absolute_local_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    text.starts_with("\\\\") || text.starts_with('/')
}

/// `user@host:path` with a non-empty user, host and path and no '/' before the ':'.
fn is_scp_like(text: &str) -> bool {
    let Some(colon) = text.find(':') else {
        return false;
    };
    let (authority, path) = (&text[..colon], &text[colon + 1..]);
    let Some(at) = authority.rfind('@') else {
        return false;
    };
    let (user, host) = (&authority[..at], &authority[at + 1..]);
    !user.is_empty()
        && !host.is_empty()
        && !path.is_empty()
        && !authority.contains('/')
        && !authority.contains('\\')
}

/// Replaces credentials embedded in URLs anywhere in `text`: web URLs lose all user info
/// (`https://user:token@host` → `https://***@host`); other schemes keep a plain user name
/// (`ssh://git@host`) but lose `user:password` pairs.
pub fn redact_credentials(text: &str) -> String {
    const SEPARATOR: &str = "://";
    let ends_authority =
        |c: char| matches!(c, '/' | '\\' | '?' | '#' | '"' | '\'') || c.is_whitespace();

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = text[cursor..].find(SEPARATOR) {
        let separator = cursor + found;
        let authority_start = separator + SEPARATOR.len();
        let authority_end = text[authority_start..]
            .char_indices()
            .find(|(_, c)| ends_authority(*c))
            .map_or(text.len(), |(offset, _)| authority_start + offset);

        let Some(at) = text[authority_start..authority_end]
            .rfind('@')
            .map(|offset| authority_start + offset)
        else {
            result.push_str(&text[cursor..authority_start]);
            cursor = authority_start;
            continue;
        };

        let scheme_start = text[..separator]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            .last()
            .map_or(separator, |(index, _)| index);
        let scheme = &text[scheme_start..separator];
        let user_info = &text[authority_start..at];
        let web = scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http");

        result.push_str(&text[cursor..authority_start]);
        result.push_str(if web || user_info.contains(':') {
            "***"
        } else {
            user_info
        });
        result.push('@');
        cursor = at + 1;
    }
    result.push_str(&text[cursor..]);
    result
}

/// The default install folder for a repository: its last path segment without ".git". `None` when
/// that is not a plain folder name.
pub fn repository_folder_name(repository: &str) -> Option<String> {
    let mut url = repository.trim();
    if let Some(cut) = url.find('?') {
        url = &url[..cut];
    }
    if let Some(cut) = url.find('#') {
        url = &url[..cut];
    }
    let url = url.trim_end_matches(['/', '\\']);

    // The last segment follows '/', '\' or, for scp-like "host:repo.git", ':'.
    let start = url.rfind(['/', '\\', ':']).map_or(0, |index| index + 1);
    let folder = strip_dot_git(&url[start..]);
    if is_safe_folder_name(folder) {
        Some(folder.to_string())
    } else {
        None
    }
}

/// One safe path segment: `^[A-Za-z0-9._-]+$`, not "." or "..".
pub fn is_safe_folder_name(folder: &str) -> bool {
    !folder.is_empty()
        && folder != "."
        && folder != ".."
        && folder
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A browsable https URL for a repository hosted on a web forge, without credentials and ".git".
pub fn web_url(repository: &str) -> Option<String> {
    let url = repository.trim();
    let strip_git =
        |path: &str| -> String { strip_dot_git(path.trim_end_matches('/')).to_string() };

    let (host_and_path, scheme) = if let Some(rest) = strip_prefix_ignore_case(url, "https://") {
        (rest, "https")
    } else if let Some(rest) = strip_prefix_ignore_case(url, "http://") {
        (rest, "http")
    } else if let Some(rest) = strip_prefix_ignore_case(url, "ssh://") {
        let rest = rest.split_once('@').map_or(rest, |(_, after)| after);
        // ssh://git@host:22/owner/repo.git: drop a port.
        let (host, path) = rest.split_once('/')?;
        let host = host.split(':').next().unwrap_or(host);
        return Some(format!("https://{host}/{}", strip_git(path)));
    } else if is_scp_like(url) {
        let (authority, path) = url.split_once(':')?;
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        return Some(format!(
            "https://{host}/{}",
            strip_git(path.trim_start_matches('/'))
        ));
    } else {
        return None;
    };

    let host_and_path = host_and_path
        .split_once('@')
        .filter(|(user, _)| !user.contains('/'))
        .map_or(host_and_path, |(_, rest)| rest);
    let (host, path) = host_and_path.split_once('/')?;
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}/{}", strip_git(path)))
}

/// The text without a trailing ".git" (any case). Index data is untrusted, so the cut is only made
/// on a char boundary: a repository such as "https://x/仓库" must not panic.
fn strip_dot_git(text: &str) -> &str {
    text.len()
        .checked_sub(4)
        .filter(|&cut| text.is_char_boundary(cut) && text[cut..].eq_ignore_ascii_case(".git"))
        .map_or(text, |cut| &text[..cut])
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    // `get` is `None` when the prefix length falls inside a multi-byte character.
    text.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

/// A short, credential-free form of a repository URL for step details: scheme, user info and a
/// trailing ".git" are dropped (`https://github.com/AstreoX/uBotSensor.git` becomes
/// `github.com/AstreoX/uBotSensor`). Mirrors `displayUrl` in the frontend. `file://` URLs and
/// absolute local paths are returned as written.
pub fn display_repository(url: &str) -> String {
    let redacted = redact_credentials(url.trim());
    for scheme in ["https://", "http://", "ssh://"] {
        if let Some(rest) = strip_prefix_ignore_case(&redacted, scheme) {
            let rest = rest
                .split_once('@')
                .filter(|(user, _)| !user.contains('/'))
                .map_or(rest, |(_, after)| after);
            return strip_dot_git(rest.trim_end_matches('/')).to_string();
        }
    }
    if redacted.contains("://") || is_absolute_local_path(&redacted) {
        return redacted;
    }
    if is_scp_like(&redacted) {
        // user@host:path
        let rest = redacted
            .split_once('@')
            .map_or(redacted.as_str(), |(_, after)| after);
        return strip_dot_git(rest.trim_end_matches('/')).to_string();
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        for url in [
            "https://github.com/AstreoX/uBotCore.git",
            "ssh://git@github.com/AstreoX/uBotCore.git",
            "git@github.com:AstreoX/uBotCore.git",
            "file:///C:/mirror/uBotCore.git",
            "C:\\mirror\\uBotCore.git",
            "D:/mirror/uBotCore.git",
            "/srv/git/uBotCore.git",
            "\\\\server\\share\\uBotCore.git",
            // `::` inside an IPv6 literal is not a remote helper (the C++ side allows it too).
            "https://[::1]/team/uBotLocal.git",
            "ssh://git@[::1]/team/uBotLocal.git",
        ] {
            assert_eq!(check_repository_url(url), UrlVerdict::Allowed, "{url}");
        }
        assert_eq!(
            check_repository_url("http://example.com/x.git"),
            UrlVerdict::Insecure
        );
        for url in [
            "",
            "-uhttps://x",
            "--upload-pack=evil",
            "ext::sh -c touch% /tmp/pwned",
            "fd::17",
            "foo+bar.baz::x",
            "git+ssh::foo",
            "::foo",
            "https://x/a b.git",
            "https://x/a\tb.git",
            "https://x/a\u{7}.git",
            "git://github.com/x.git",
            "ftp://example.com/x.git",
            "relative/path.git",
            "https://",
            "host:path",
            "https://-oProxyCommand=x/y",
        ] {
            assert!(
                !check_repository_url(url).is_allowed(),
                "{url:?} is rejected"
            );
        }
    }

    #[test]
    fn binary_url_policy() {
        for url in [
            "https://github.com/AstreoX/uBotSensor/releases/download/v0.1.0/UBotSensor-0.1.0-UE5.5-Win64.zip",
            "HTTPS://h/x.zip",
            "file:///C:/mirror/x.zip",
            "file://server/share/x.zip",
            "C:\\mirror\\x.zip",
            "D:/mirror/x.zip",
            "\\\\server\\share\\x.zip",
            "/srv/mirror/x.zip",
        ] {
            assert_eq!(check_binary_url(url), Ok(()), "{url}");
        }
        for (url, reason) in [
            ("http://example.com/x.zip", "plain http://"),
            ("http://127.0.0.1:8080/x.zip", "plain http://"),
            ("ssh://git@h/x.zip", "https://"),
            ("git@h:x.zip", "https://"),
            ("relative/x.zip", "https://"),
            ("ftp://h/x.zip", "not allowed"),
            (
                "https://evil.example.net\\@trusted.example.com/a/b.zip",
                "backslash",
            ),
            ("https://h\\x/y.zip", "backslash"),
            ("-https://h/x.zip", "'-'"),
            ("https://h/a b.zip", "whitespace"),
            ("ext::sh -c x", "whitespace"),
            ("ext::x", "remote helper"),
            ("", "empty"),
        ] {
            let error = check_binary_url(url).unwrap_err();
            assert!(error.contains(reason), "{url}: {error}");
        }
    }

    #[test]
    fn download_sources_show_host_and_file_only() {
        assert_eq!(
            display_download_source(
                "https://github.com/AstreoX/uBotSensor/releases/download/v0.1.0/UBotSensor-0.1.0-UE5.5-Win64.zip"
            ),
            "github.com/…/UBotSensor-0.1.0-UE5.5-Win64.zip"
        );
        assert_eq!(
            display_download_source("https://user:SECRET@h.example/x.zip?token=SECRET#f"),
            "h.example/x.zip"
        );
        assert_eq!(display_download_source("https://h.example"), "h.example");
        assert_eq!(
            display_download_source("http://127.0.0.1:8080/a/b.zip"),
            "127.0.0.1:8080/…/b.zip"
        );
        assert_eq!(
            display_download_source("file:///C:/mirror/x.zip"),
            "C:/mirror/x.zip"
        );
        assert_eq!(
            display_download_source("file://server/share/x.zip"),
            "//server/share/x.zip"
        );
        assert_eq!(
            display_download_source("D:\\mirror\\x.zip"),
            "D:\\mirror\\x.zip"
        );
        // The host shown is the one the downloader's parser finds, whatever the text looks like.
        assert_eq!(
            display_download_source("https://evil.example.net\\@trusted.example.com/a/b.zip"),
            "evil.example.net/…/b.zip"
        );
        assert_eq!(
            display_download_source("https://trusted.example.com@evil.example.net/a/b.zip"),
            "evil.example.net/…/b.zip"
        );
        assert_eq!(
            display_download_source("HTTPS://User:Pw@Host.Example:8443/x.zip"),
            "host.example:8443/x.zip"
        );
        assert_eq!(
            display_download_source("https://[::1]:9/a/x.zip"),
            "[::1]:9/…/x.zip"
        );
        // Untrusted text never panics.
        let _ = display_download_source("https://仓/😀.zip");
        let _ = display_download_source("file://");
        let _ = display_download_source("https://");
        let _ = display_download_source("https://:::/x");
    }

    #[test]
    fn stored_urls_lose_secrets() {
        assert_eq!(
            strip_url_secrets("https://user:SECRETPW@h.example/dl/x.zip?sig=SECRETSIG&x=1#frag"),
            "https://***@h.example/dl/x.zip"
        );
        assert_eq!(
            strip_url_secrets("https://h.example/dl/x.zip"),
            "https://h.example/dl/x.zip"
        );
        assert_eq!(
            strip_url_secrets("file:///E:/mirror/x.zip?sig=SECRETSIG"),
            "file:///E:/mirror/x.zip"
        );
        // A Windows file name may hold '#' and '?'-free names; a local path is kept as written.
        assert_eq!(
            strip_url_secrets("D:\\mirror\\x #1.zip"),
            "D:\\mirror\\x #1.zip"
        );
        assert_eq!(strip_url_secrets("/srv/m#1/x.zip"), "/srv/m#1/x.zip");
    }

    #[test]
    fn redacts_credentials() {
        assert_eq!(
            redact_credentials("https://user:token@github.com/a/b.git"),
            "https://***@github.com/a/b.git"
        );
        assert_eq!(
            redact_credentials("https://ghp_abc@github.com/a/b.git"),
            "https://***@github.com/a/b.git"
        );
        assert_eq!(
            redact_credentials("ssh://git@github.com/a/b.git"),
            "ssh://git@github.com/a/b.git"
        );
        assert_eq!(
            redact_credentials("ssh://git:secret@github.com/a/b.git"),
            "ssh://***@github.com/a/b.git"
        );
        assert_eq!(
            redact_credentials("https://github.com/a/b@c.git"),
            "https://github.com/a/b@c.git"
        );
        assert_eq!(
            redact_credentials("Cloning into 'x'... from https://a:b@h/x and http://c@h/y."),
            "Cloning into 'x'... from https://***@h/x and http://***@h/y."
        );
        assert_eq!(redact_credentials("no urls here"), "no urls here");
        assert_eq!(
            redact_credentials("fatal: 'https://u:p@host' ok"),
            "fatal: 'https://***@host' ok"
        );
    }

    #[test]
    fn folder_names() {
        assert_eq!(
            repository_folder_name("https://github.com/AstreoX/uBotSensor.git").as_deref(),
            Some("uBotSensor")
        );
        assert_eq!(
            repository_folder_name("git@github.com:AstreoX/uBotROS.git/").as_deref(),
            Some("uBotROS")
        );
        assert_eq!(
            repository_folder_name("git@host:repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            repository_folder_name("E:\\tmp\\bare\\uBotCore.git").as_deref(),
            Some("uBotCore")
        );
        assert_eq!(
            repository_folder_name("https://x/y/z.GIT?ref=1#frag").as_deref(),
            Some("z")
        );
        assert_eq!(repository_folder_name("https://x/.."), None);
        assert_eq!(repository_folder_name("https://x/.git"), None);
        assert_eq!(repository_folder_name(""), None);
        assert!(is_safe_folder_name("uBot.Core-1_2"));
        assert!(!is_safe_folder_name("a b"));
        assert!(!is_safe_folder_name("."));
    }

    #[test]
    fn web_urls() {
        assert_eq!(
            web_url("https://github.com/AstreoX/uBotCore.git").as_deref(),
            Some("https://github.com/AstreoX/uBotCore")
        );
        assert_eq!(
            web_url("https://user:tok@github.com/AstreoX/uBotCore").as_deref(),
            Some("https://github.com/AstreoX/uBotCore")
        );
        assert_eq!(
            web_url("git@github.com:AstreoX/uBotCore.git").as_deref(),
            Some("https://github.com/AstreoX/uBotCore")
        );
        assert_eq!(
            web_url("ssh://git@github.com:22/AstreoX/uBotCore.git").as_deref(),
            Some("https://github.com/AstreoX/uBotCore")
        );
        assert_eq!(web_url("C:\\mirror\\x.git"), None);
        assert_eq!(web_url("file:///C:/x.git"), None);
    }

    #[test]
    fn non_ascii_repositories_do_not_panic() {
        // Index data is untrusted: every helper must cope with multi-byte characters at the
        // places where it cuts ".git" or a scheme.
        assert_eq!(repository_folder_name("https://x/仓库"), None);
        assert_eq!(repository_folder_name("https://x/仓库.git"), None);
        assert_eq!(repository_folder_name("仓库"), None);
        assert_eq!(web_url("仓库地址"), None);
        assert_eq!(
            web_url("https://github.com/AstreoX/仓库").as_deref(),
            Some("https://github.com/AstreoX/仓库")
        );
        assert_eq!(
            web_url("https://github.com/AstreoX/仓库.git").as_deref(),
            Some("https://github.com/AstreoX/仓库")
        );
        assert_eq!(
            web_url("ssh://git@github.com/AstreoX/仓.git").as_deref(),
            Some("https://github.com/AstreoX/仓")
        );
        assert_eq!(display_repository("仓库地址"), "仓库地址");
        assert_eq!(display_repository("https://x/仓库.git"), "x/仓库");
        let _ = check_repository_url("仓库地址");
        let _ = redact_credentials("https://仓@h/😀.git");
    }

    #[test]
    fn display_forms() {
        assert_eq!(
            display_repository("https://github.com/AstreoX/uBotSensor.git"),
            "github.com/AstreoX/uBotSensor"
        );
        assert_eq!(display_repository("https://user:tok@h/x.git"), "h/x");
        assert_eq!(display_repository("http://h/a/b.GIT/"), "h/a/b");
        assert_eq!(display_repository("ssh://git@h/a/b.git"), "h/a/b");
        assert_eq!(
            display_repository("git@github.com:AstreoX/uBotROS.git"),
            "github.com:AstreoX/uBotROS"
        );
        assert_eq!(
            display_repository("file:///C:/mirror/uBotCore.git"),
            "file:///C:/mirror/uBotCore.git"
        );
        assert_eq!(
            display_repository("E:\\tmp\\bare\\uBotCore.git"),
            "E:\\tmp\\bare\\uBotCore.git"
        );
    }
}
