//! Install-integrity preflight (`docs/install_integrity_preflight_card.md`
//! §B1–B3): detect a hand-rolled (non-package-owned) v3 tree.
//!
//! REPORT/health DEGRADE only — a WARN "unsupported", never a BLOCK: a
//! hand-rolled box that otherwise validates KEEPS shaping (card §B3, §0/§7).
//! Scope = a DEFINED file set (the bridge binary + its unit file), NOT a
//! package-wide verify (which false-positives on operator-tuned config).

/// The DEFINED integrity set — the binary + the bridge unit. BOTH belong to
/// the SAME package (`libreqos-dataplane`, `packaging/build_bridge_artifact.sh`),
/// which is what makes the single `dpkg -V <package>` verify sound; expanding
/// the set to units from OTHER packages would require a per-package verify, so
/// it stays the MINIMAL single-package ANCHOR (the binary anchors integrity,
/// the unit anchors boot wiring).
pub const BRIDGE_BINARY: &str = "/usr/libexec/libreqos/bridge_rs";
pub const BRIDGE_UNIT: &str = "/etc/systemd/system/libreqos-bridge.service";
pub const INTEGRITY_SET: &[&str] = &[BRIDGE_BINARY, BRIDGE_UNIT];

/// The dpkg seam (the card's `dpkg-query` seam): the pure core never shells
/// out — the standalone binary supplies [`CommandDpkg`], tests supply fakes.
pub trait DpkgQuery {
    /// `dpkg-query -S <path>` → the owning package, if any.
    fn owner_of(&self, path: &str) -> Option<String>;
    /// The MODIFIED files for `package` (empty = intact); `None` = no verify
    /// tool (the no-`debsums` case → the version-stamp fallback).
    fn verify(&self, package: &str) -> Option<Vec<String>>;
}

/// The install-integrity report (card §B1 two-step).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallIntegrityReport {
    /// Step 1: some package owns a member of the defined set.
    pub package_owned: bool,
    /// Step 2: the defined set is intact. `None` = could not verify (treated
    /// as intact — never a false hand-rolled flag).
    pub defined_set_intact: Option<bool>,
    /// The owning package, when known.
    pub package: Option<String>,
}

impl InstallIntegrityReport {
    /// The box is a supported (package-owned, unmodified) v3.
    pub fn is_supported(&self) -> bool {
        self.package_owned && self.defined_set_intact != Some(false)
    }

    /// The report/health degrade message (card §B3), or `None` when supported.
    pub fn warning(&self) -> Option<String> {
        if !self.package_owned {
            return Some(
                "the running v3 tree is NOT package-owned (a hand-rolled install) — unsupported \
                 by the shipped artifact's upgrade/preflight assumptions; shaping continues, but \
                 package upgrades may be unsafe"
                    .to_string(),
            );
        }
        match self.defined_set_intact {
            Some(false) => Some(format!(
                "the package-owned v3 tree is MODIFIED within the integrity set ({}); shaping \
                 continues, but the running artifact is not the shipped one",
                INTEGRITY_SET.join(", ")
            )),
            _ => None,
        }
    }
}

/// Assess install integrity (pure over the seam).
pub fn assess(query: &impl DpkgQuery, set: &[&str]) -> InstallIntegrityReport {
    let Some(package) = set.iter().find_map(|path| query.owner_of(path)) else {
        return InstallIntegrityReport {
            package_owned: false,
            defined_set_intact: None,
            package: None,
        };
    };
    // Package-wide verify flags legitimately operator-tuned files; intersecting
    // with the DEFINED set removes that false positive (card §B1).
    let defined_set_intact = query
        .verify(&package)
        .map(|modified| !modified.iter().any(|path| set.contains(&path.as_str())));
    InstallIntegrityReport {
        package_owned: true,
        defined_set_intact,
        package: Some(package),
    }
}

/// The real dpkg seam for the standalone binary.
pub struct CommandDpkg;

impl DpkgQuery for CommandDpkg {
    fn owner_of(&self, path: &str) -> Option<String> {
        let out = std::process::Command::new("dpkg-query")
            .args(["-S", path])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?;
        // `pkg: /path` (or `a, b: /path` for a shared file). Split the OWNERS
        // from the path at the LAST `": "`, then take the first owner (a
        // unique binary path normally has exactly one).
        let owners = text
            .rsplit_once(": ")
            .map(|(owners, _path)| owners)
            .unwrap_or_else(|| text.trim());
        owners
            .split(',')
            .map(str::trim)
            .find(|name| !name.is_empty())
            .map(str::to_string)
    }

    fn verify(&self, package: &str) -> Option<Vec<String>> {
        let out = std::process::Command::new("dpkg")
            .args(["-V", package])
            .output()
            .ok()?;
        // dpkg -V prints one line per modified file; the path is the last
        // whitespace token. An empty body = intact.
        let text = String::from_utf8_lossy(&out.stdout);
        Some(
            text.lines()
                .filter_map(|line| line.split_whitespace().last().map(str::to_string))
                .filter(|path| path.starts_with('/'))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        owner: Option<String>,
        modified: Option<Vec<String>>,
    }

    impl DpkgQuery for Fake {
        fn owner_of(&self, _path: &str) -> Option<String> {
            self.owner.clone()
        }
        fn verify(&self, _package: &str) -> Option<Vec<String>> {
            self.modified.clone()
        }
    }

    /// A hand-rolled box: no package owns the set → unsupported + a WARN.
    #[test]
    fn a_hand_rolled_tree_is_unsupported() {
        let report = assess(
            &Fake {
                owner: None,
                modified: None,
            },
            INTEGRITY_SET,
        );
        assert!(!report.package_owned);
        assert!(!report.is_supported());
        let warning = report.warning().expect("a hand-rolled tree warns");
        assert!(warning.contains("NOT package-owned"), "{warning}");
    }

    /// A packaged, intact box is supported (no warning).
    #[test]
    fn a_packaged_intact_tree_is_supported() {
        let report = assess(
            &Fake {
                owner: Some("libreqos".to_string()),
                modified: Some(Vec::new()),
            },
            INTEGRITY_SET,
        );
        assert!(report.package_owned);
        assert!(report.is_supported());
        assert!(report.warning().is_none());
    }

    /// A modified file INSIDE the set is unsupported (the running artifact is
    /// not the shipped one).
    #[test]
    fn a_modified_set_member_is_unsupported() {
        let report = assess(
            &Fake {
                owner: Some("libreqos".to_string()),
                modified: Some(vec![BRIDGE_BINARY.to_string()]),
            },
            INTEGRITY_SET,
        );
        assert!(!report.is_supported());
        assert!(report.warning().expect("warns").contains("MODIFIED"));
    }

    /// A modified OPERATOR FILE outside the set is NOT a false positive
    /// (card §B1: a legitimately-tuned config must not flag).
    #[test]
    fn a_modified_operator_file_outside_the_set_is_supported() {
        let report = assess(
            &Fake {
                owner: Some("libreqos".to_string()),
                modified: Some(vec!["/etc/libreqos/lqos.conf".to_string()]),
            },
            INTEGRITY_SET,
        );
        assert!(report.is_supported());
        assert!(report.warning().is_none());
    }

    /// No verify tool (the no-`debsums` case) is treated as intact, never a
    /// false hand-rolled flag.
    #[test]
    fn an_unavailable_verify_tool_is_not_a_failure() {
        let report = assess(
            &Fake {
                owner: Some("libreqos".to_string()),
                modified: None,
            },
            INTEGRITY_SET,
        );
        assert!(report.is_supported());
        assert_eq!(report.defined_set_intact, None);
    }
}
