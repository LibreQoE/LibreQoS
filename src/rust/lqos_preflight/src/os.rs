//! The OS-floor check: the v3 packages are built against glibc 2.39 (Ubuntu
//! 24.04 "noble"), so a pre-noble host must be told plainly before the install
//! fails at runtime with an unhelpful `GLIBC_2.39 not found`.
//!
//! Read-only and static-safe: it parses `/etc/os-release` (a file read), with
//! no subprocess and no network — the same surface as the rest of the checker.

/// The Ubuntu release the v3 packages are qualified on (the glibc-2.39 floor).
pub const UBUNTU_FLOOR: (u32, u32) = (24, 4);

/// The parsed `/etc/os-release` facts the floor check needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OsRelease {
    /// The `ID` field (e.g. `ubuntu`).
    pub id: Option<String>,
    /// The `VERSION_ID` field (e.g. `24.04`).
    pub version_id: Option<String>,
    /// The human-readable `PRETTY_NAME`, when present.
    pub pretty_name: Option<String>,
}

impl OsRelease {
    /// A human label for messages (`PRETTY_NAME`, else `ID`, else a fallback).
    pub fn label(&self) -> String {
        self.pretty_name
            .clone()
            .or_else(|| self.id.clone())
            .unwrap_or_else(|| "this system".to_string())
    }

    /// True when the `ID` is Ubuntu (case-insensitive).
    pub fn is_ubuntu(&self) -> bool {
        self.id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case("ubuntu"))
    }
}

/// Parse the `KEY=value` lines of `/etc/os-release`; surrounding single or
/// double quotes are stripped, and only the fields the floor check needs are
/// collected (unknown keys are ignored).
pub fn parse_os_release(text: &str) -> OsRelease {
    let mut release = OsRelease::default();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        match key {
            "ID" => release.id = Some(value),
            "VERSION_ID" => release.version_id = Some(value),
            "PRETTY_NAME" => release.pretty_name = Some(value),
            _ => {}
        }
    }
    release
}

/// Parse a dotted numeric version prefix into `(major, minor)`; a missing minor
/// counts as zero (`"24"` -> `(24, 0)`).
pub fn parse_version(value: &str) -> Option<(u32, u32)> {
    let mut parts = value.split('.').filter(|part| !part.is_empty());
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts
        .next()
        .and_then(|part| part.trim().parse().ok())
        .unwrap_or(0);
    Some((major, minor))
}

/// The plain-language OS-floor block, or `None` when the host meets the floor.
///
/// A host that is not Ubuntu, is an Ubuntu older than 24.04, or whose release
/// cannot be read is blocked with the same "Ubuntu 24.04+ required" guidance.
pub fn floor_block(release: &OsRelease) -> Option<String> {
    if !release.is_ubuntu() {
        return Some(format!(
            "Ubuntu 24.04+ required — LibreQoS v3 is built against glibc 2.39 and \
             qualified on Ubuntu Server 24.04 LTS or newer; {} is not a supported \
             platform (packaging/INSTALL.md)",
            release.label()
        ));
    }
    let version_id = release.version_id.as_deref();
    match version_id.and_then(parse_version) {
        Some(version) if version >= UBUNTU_FLOOR => None,
        Some(_) => Some(format!(
            "Ubuntu 24.04+ required — this host is Ubuntu {}, which predates the \
             glibc 2.39 that LibreQoS v3 is built against; upgrade to Ubuntu Server \
             24.04 LTS or newer before installing (packaging/INSTALL.md)",
            version_id.unwrap_or("(unknown)")
        )),
        None => Some(format!(
            "Ubuntu 24.04+ required — could not read VERSION_ID from /etc/os-release \
             (found {}); LibreQoS v3 needs glibc 2.39 (Ubuntu Server 24.04 LTS or \
             newer) (packaging/INSTALL.md)",
            release.label()
        )),
    }
}

/// The Noble systemd-hotfix advisory, or `None` on a host where it does not
/// apply. On Ubuntu 24.04 (Noble) a host using `systemd-networkd` needs the
/// LibreQoS systemd hotfix before the v3 `postinst` can start services (the
/// install otherwise hard-stops).
///
/// This is an ADVISORY only: it performs no probe and does not affect the
/// verdict, so a stock Noble host that does not use `systemd-networkd` is not
/// warned falsely.
pub fn hotfix_advisory(release: &OsRelease) -> Option<String> {
    if !release.is_ubuntu() {
        return None;
    }
    let version = release.version_id.as_deref().and_then(parse_version)?;
    if version.0 != 24 || version.1 != 4 {
        return None;
    }
    Some(
        "on Ubuntu 24.04 (Noble), if this host runs systemd-networkd it needs the \
         LibreQoS systemd hotfix before the v3 install can start services: run \
         'sudo /opt/libreqos/src/systemd_hotfix.sh install' first \
         (packaging/INSTALL.md)"
            .to_string(),
    )
}

/// Read the host's `/etc/os-release`; an unreadable file yields an empty
/// release, which [`floor_block`] blocks.
pub fn check() -> OsRelease {
    check_at("/etc/os-release")
}

/// The testable surface: parse a specific os-release file.
pub fn check_at(path: &str) -> OsRelease {
    std::fs::read_to_string(path)
        .map(|text| parse_os_release(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOBLE: &str =
        "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\n";
    const JAMMY: &str = "PRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\nVERSION_ID=\"22.04\"\nID=ubuntu\n";
    const DEBIAN: &str =
        "PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nVERSION_ID=\"13\"\nID=debian\n";

    #[test]
    fn parses_the_fields_and_strips_quotes() {
        let release = parse_os_release(NOBLE);
        assert_eq!(release.id.as_deref(), Some("ubuntu"));
        assert_eq!(release.version_id.as_deref(), Some("24.04"));
        assert_eq!(release.pretty_name.as_deref(), Some("Ubuntu 24.04.4 LTS"));
    }

    #[test]
    fn parses_dotted_and_bare_versions() {
        assert_eq!(parse_version("24.04"), Some((24, 4)));
        assert_eq!(parse_version("24"), Some((24, 0)));
        assert_eq!(parse_version("24.04.4 LTS"), Some((24, 4)));
        assert_eq!(parse_version("nonsense"), None);
    }

    #[test]
    fn ubuntu_at_or_above_the_floor_passes() {
        assert_eq!(floor_block(&parse_os_release(NOBLE)), None);
        let newer = parse_os_release("ID=ubuntu\nVERSION_ID=\"26.04\"\n");
        assert_eq!(floor_block(&newer), None);
    }

    #[test]
    fn an_older_ubuntu_blocks_with_the_plain_message() {
        let block = floor_block(&parse_os_release(JAMMY)).expect("22.04 blocks");
        assert!(block.starts_with("Ubuntu 24.04+ required"));
        assert!(block.contains("Ubuntu 22.04"));
    }

    #[test]
    fn a_non_ubuntu_platform_blocks() {
        let block = floor_block(&parse_os_release(DEBIAN)).expect("debian blocks");
        assert!(block.starts_with("Ubuntu 24.04+ required"));
        assert!(block.contains("Debian GNU/Linux 13"));
    }

    #[test]
    fn the_noble_hotfix_advisory_is_noble_only() {
        let note = hotfix_advisory(&parse_os_release(NOBLE)).expect("noble advises");
        assert!(note.contains("systemd_hotfix.sh install"));
        // Every other release (newer, older, non-Ubuntu, unreadable) is silent:
        // the advisory performs no probe and must not become a false positive.
        for text in [
            "ID=ubuntu\nVERSION_ID=\"26.04\"\n",
            "ID=ubuntu\nVERSION_ID=\"22.04\"\n",
            DEBIAN,
            "",
        ] {
            assert!(hotfix_advisory(&parse_os_release(text)).is_none());
        }
    }

    #[test]
    fn an_unreadable_release_blocks() {
        let block = floor_block(&OsRelease::default()).expect("empty blocks");
        assert!(block.starts_with("Ubuntu 24.04+ required"));
    }
}
