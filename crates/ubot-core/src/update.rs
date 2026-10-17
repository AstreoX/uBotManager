//! Looking for a newer uBot Manager (SPEC §6 `updates`, §7 `check_for_update`).
//!
//! One HTTPS request to the GitHub Releases API for the latest published release; nothing is
//! downloaded or installed. The reply is reduced to a version: the release page is rebuilt from it
//! rather than taken from the reply, and drafts and prereleases are ignored.

use serde::Deserialize;

use crate::model::UpdateInfo;
use crate::sources::Fetcher;
use crate::version::SemVer;

/// The latest published (not draft, not prerelease) release of uBot Manager.
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/AstreoX/uBotManager/releases/latest";
/// The release page of a tag: this plus the tag name.
pub const RELEASE_PAGE_PREFIX: &str = "https://github.com/AstreoX/uBotManager/releases/tag/";

/// The version this build was compiled as.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Asks GitHub for the latest release and compares it with `current`.
pub fn check(fetcher: &dyn Fetcher, current: &str) -> Result<UpdateInfo, String> {
    let body = fetcher.fetch(LATEST_RELEASE_API)?;
    parse_latest(&body, current)
}

/// Reads the reply of [`LATEST_RELEASE_API`]. The tag must be `v` plus a release version, the
/// form the release workflow creates; anything else is refused rather than guessed at.
pub fn parse_latest(body: &str, current: &str) -> Result<UpdateInfo, String> {
    let running = SemVer::parse(current)
        .ok_or_else(|| format!("the running version \"{current}\" is not a version"))?;
    let release: Release = serde_json::from_str(body)
        .map_err(|error| format!("the reply of GitHub is not a release: {error}"))?;
    if release.draft || release.prerelease {
        return Err("the latest release is a draft or a prerelease".into());
    }
    let latest = release
        .tag_name
        .strip_prefix('v')
        .and_then(SemVer::parse)
        .filter(|version| !version.is_prerelease())
        .ok_or_else(|| {
            format!(
                "the tag \"{}\" of the latest release is not v<version>",
                release.tag_name.chars().take(40).collect::<String>()
            )
        })?;
    if format!("v{latest}") != release.tag_name {
        return Err(format!(
            "the tag \"{}\" of the latest release is not v<version>",
            release.tag_name.chars().take(40).collect::<String>()
        ));
    }
    Ok(UpdateInfo {
        current: running.to_string(),
        latest: latest.to_string(),
        available: latest.compare(&running) == std::cmp::Ordering::Greater,
        url: format!("{RELEASE_PAGE_PREFIX}{}", release.tag_name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Result<String, String>);

    impl Fetcher for Fixed {
        fn fetch(&self, url: &str) -> Result<String, String> {
            assert_eq!(url, LATEST_RELEASE_API);
            self.0.clone()
        }
    }

    fn release(tag: &str) -> String {
        format!(r#"{{ "tag_name": "{tag}", "draft": false, "prerelease": false, "name": "x" }}"#)
    }

    #[test]
    fn a_newer_release_is_available() {
        let info = check(&Fixed(Ok(release("v0.1.2"))), "0.1.1").unwrap();
        assert_eq!(info.current, "0.1.1");
        assert_eq!(info.latest, "0.1.2");
        assert!(info.available);
        assert_eq!(
            info.url,
            "https://github.com/AstreoX/uBotManager/releases/tag/v0.1.2"
        );
    }

    #[test]
    fn the_same_or_an_older_release_is_not() {
        for latest in ["v0.1.1", "v0.1.0", "v0.0.9"] {
            let info = parse_latest(&release(latest), "0.1.1").unwrap();
            assert!(!info.available, "{latest}");
        }
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(
            parse_latest(&release("v0.10.0"), "0.9.0")
                .unwrap()
                .available
        );
        assert!(
            parse_latest(&release("v1.0.0"), "0.99.99")
                .unwrap()
                .available
        );
        assert!(
            !parse_latest(&release("v0.9.0"), "0.10.0")
                .unwrap()
                .available
        );
    }

    #[test]
    fn a_running_prerelease_is_older_than_its_release() {
        assert!(
            parse_latest(&release("v0.2.0"), "0.2.0-beta.1")
                .unwrap()
                .available
        );
    }

    #[test]
    fn drafts_prereleases_and_odd_tags_are_refused() {
        let draft = r#"{ "tag_name": "v9.9.9", "draft": true }"#;
        let pre = r#"{ "tag_name": "v9.9.9", "prerelease": true }"#;
        assert!(parse_latest(draft, "0.1.1").is_err());
        assert!(parse_latest(pre, "0.1.1").is_err());
        for tag in [
            "0.1.2",
            "v0.1.2-beta.1",
            "release-2",
            "v",
            "",
            "v0.1.2.3",
            "V0.1.2",
            "v0.1",
        ] {
            assert!(parse_latest(&release(tag), "0.1.1").is_err(), "{tag}");
        }
    }

    #[test]
    fn a_reply_that_is_not_a_release_is_an_error() {
        assert!(parse_latest("not json", "0.1.1").is_err());
        assert!(parse_latest("{}", "0.1.1").is_err());
        assert!(parse_latest(r#"{ "message": "rate limit exceeded" }"#, "0.1.1").is_err());
    }

    #[test]
    fn the_page_is_built_from_the_version_not_taken_from_the_reply() {
        let body = r#"{ "tag_name": "v0.2.0", "html_url": "https://evil.example/x" }"#;
        let info = parse_latest(body, "0.1.1").unwrap();
        assert!(info.url.starts_with(RELEASE_PAGE_PREFIX));
        assert!(!info.url.contains("evil"));
    }

    #[test]
    fn fetch_errors_pass_through() {
        let error = check(&Fixed(Err("could not connect".into())), "0.1.1").unwrap_err();
        assert_eq!(error, "could not connect");
    }

    /// A real reply of the API (GitHub, 2026-10-17, release 0.1.1).
    const REAL_REPLY: &str = include_str!("../tests/fixtures/github-latest-release.json");

    #[test]
    fn reads_a_real_reply() {
        let newer = parse_latest(REAL_REPLY, "0.1.0").unwrap();
        assert_eq!(newer.latest, "0.1.1");
        assert!(newer.available);
        assert_eq!(
            newer.url,
            "https://github.com/AstreoX/uBotManager/releases/tag/v0.1.1"
        );
        assert!(!parse_latest(REAL_REPLY, "0.1.1").unwrap().available);
    }

    #[test]
    fn the_running_version_is_a_version() {
        assert!(SemVer::parse(current_version()).is_some());
    }
}
