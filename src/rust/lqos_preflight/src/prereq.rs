//! The card's §1.4 split-fetch prerequisites.
//!
//! After the split, the v3 package ships NO dataplane payload: the installer
//! fetches it from the download portal (`src/src/update_bridge.sh`, the
//! `LQOS_DATAPLANE_URL` hook). This module checks that the URL is configured
//! and well-formed.
//!
//! Reachability is deliberately NOT probed here: a DNS/NSS call would break
//! the static-safe syscall surface (the ruling's no-dns/no-nss rule), and
//! `update_bridge.sh` is already graceful on an unreachable endpoint — the
//! install stays non-bricking and the bridge simply stays inactive.

use std::process::Command;
use std::time::Duration;

/// The default artifact URL (the public download portal).
pub const DEFAULT_DATAPLANE_URL: &str = "https://download.libreqos.com/dataplane.tar.gz";

/// Parse `scheme://host[:port]/…` into `(host, port)`.
pub fn parse_endpoint(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let default_port = match scheme {
        "https" => 443,
        "http" => 80,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?; // drop any userinfo
    if host_port.is_empty() {
        return None;
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            Some((host.to_string(), port.parse().ok()?))
        }
        Some(_) => None,
        None => Some((host_port.to_string(), default_port)),
    }
}

/// The resolved split-fetch prerequisite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchPrereq {
    /// The configured artifact URL.
    pub url: String,
    /// The parsed `(host, port)`, when the URL is well-formed.
    pub endpoint: Option<(String, u16)>,
}

impl FetchPrereq {
    /// True when the URL is configured and well-formed.
    pub fn is_configured(&self) -> bool {
        self.endpoint.is_some()
    }
}

/// Read the configured artifact URL (`LQOS_DATAPLANE_URL` or the default) and
/// parse it. No network access.
pub fn check_fetch() -> FetchPrereq {
    check_fetch_url(&configured_url())
}

/// Parse a specific artifact URL (the testable surface).
pub fn check_fetch_url(url: &str) -> FetchPrereq {
    FetchPrereq {
        url: url.to_string(),
        endpoint: parse_endpoint(url),
    }
}

fn configured_url() -> String {
    std::env::var("LQOS_DATAPLANE_URL").unwrap_or_else(|_| DEFAULT_DATAPLANE_URL.to_string())
}

/// A connect timeout kept for callers that do probe (not used by the static
/// build; the update hook owns reachability).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The default `sources.list.d` entry the fetch hook writes.
pub const DEFAULT_REPO_SOURCE: &str = "/etc/apt/sources.list.d/libreqos.list";

/// The apt archive the dataplane package comes from (the repo-check arm).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoConfig {
    /// The checked `sources.list.d` entry.
    pub source_file: String,
    /// The parsed archive URL, when the entry exists and is well-formed.
    pub url: Option<String>,
    /// The parsed suite, when present.
    pub suite: Option<String>,
    /// The candidate version reported by `apt-cache policy`, when available.
    pub candidate: Option<String>,
}

impl RepoConfig {
    /// True when the archive entry parses (a URL + a suite).
    pub fn is_configured(&self) -> bool {
        self.url.is_some() && self.suite.is_some()
    }
}

/// Parse a one-line apt source entry (`deb [opts] URL suite component…`) into
/// `(url, suite)`. Returns `None` for comments, blank lines, or non-`deb`
/// lines.
pub fn parse_deb_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let rest = line.strip_prefix("deb ")?;
    // Drop the optional `[opt=… opt2=…]` bracket group.
    let rest = rest.trim_start();
    let rest = if let Some(after) = rest.strip_prefix('[') {
        let close = after.find(']')?;
        after[close + 1..].trim_start()
    } else {
        rest
    };
    let mut fields = rest.split_whitespace();
    let url = fields.next()?.to_string();
    let suite = fields.next()?.to_string();
    Some((url, suite))
}

/// The `apt-cache policy` candidate for one package (a local query, no
/// network). Subprocesses are within the preflight's static-safe surface.
pub fn apt_candidate(package: &str) -> Option<String> {
    let out = Command::new("apt-cache").args(["policy", package]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Candidate:")
            .map(|value| value.trim().to_string())
    })
}

/// Read the archive entry (and the candidate version) for the repo-check arm.
pub fn check_repo() -> RepoConfig {
    check_repo_at(DEFAULT_REPO_SOURCE, "libreqos-dataplane")
}

/// The testable surface: check a specific source file + package.
pub fn check_repo_at(source_file: &str, package: &str) -> RepoConfig {
    let (url, suite) = std::fs::read_to_string(source_file)
        .ok()
        .and_then(|text| text.lines().find_map(parse_deb_line))
        .map(|(url, suite)| (Some(url), Some(suite)))
        .unwrap_or((None, None));
    RepoConfig {
        source_file: source_file.to_string(),
        url,
        suite,
        candidate: apt_candidate(package),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_default_port() {
        assert_eq!(
            parse_endpoint("https://download.libreqos.com/dataplane.tar.gz"),
            Some(("download.libreqos.com".to_string(), 443))
        );
    }

    #[test]
    fn parses_explicit_port_and_userinfo() {
        assert_eq!(
            parse_endpoint("http://user:pass@portal.internal:8080/x"),
            Some(("portal.internal".to_string(), 8080))
        );
    }

    #[test]
    fn rejects_malformed_urls() {
        assert_eq!(parse_endpoint("ftp://host/x"), None);
        assert_eq!(parse_endpoint("not a url"), None);
        assert_eq!(parse_endpoint("https://"), None);
    }

    #[test]
    fn check_fetch_url_reports_configuration() {
        assert!(check_fetch_url(DEFAULT_DATAPLANE_URL).is_configured());
        assert!(!check_fetch_url("nonsense").is_configured());
    }

    #[test]
    fn parses_apt_source_lines() {
        assert_eq!(
            parse_deb_line("deb [signed-by=/usr/share/keyrings/x.gpg] https://repo.libreqos.com stable main"),
            Some(("https://repo.libreqos.com".to_string(), "stable".to_string()))
        );
        assert_eq!(
            parse_deb_line("deb https://repo.libreqos.com beta main"),
            Some(("https://repo.libreqos.com".to_string(), "beta".to_string()))
        );
    }

    #[test]
    fn ignores_comments_and_non_deb_lines() {
        assert_eq!(parse_deb_line("# a comment"), None);
        assert_eq!(parse_deb_line(""), None);
        assert_eq!(parse_deb_line("deb-src https://x stable main"), None);
        assert_eq!(parse_deb_line("deb"), None);
    }
}
