//! The control-seam alignment assert.
//!
//! An installed `lqosd` built against a different wire `PROTOCOL_VERSION` than
//! the dataplane artifact rejects the bridge at attach — the observed
//! `Bridge seam attach failed: magic/version mismatch`. The preflight compares
//! the two so the operator sees the upgrade-ordering warning BEFORE
//! installing, instead of a silently dead bridge afterwards.

use std::path::Path;
use std::process::Command;

/// The protocol this build (and the shipped dataplane artifact) speaks.
pub const EXPECTED_PROTOCOL_VERSION: u32 = lqos_dataplane_types::wire::PROTOCOL_VERSION;

/// The canonical install path of the control-plane daemon.
pub const INSTALLED_LQOSD: &str = "/usr/libexec/libreqos/lqosd";

/// The result of the seam comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeamReport {
    /// The installed `lqosd`'s protocol, when it could be read.
    pub installed: Option<u32>,
    /// The protocol this preflight expects.
    pub expected: u32,
    /// True when the two agree, or the check was not applicable (no install).
    pub aligned: bool,
}

/// Compare the installed protocol against the expected one (pure).
pub fn assess(installed: Option<u32>, expected: u32) -> SeamReport {
    SeamReport {
        installed,
        expected,
        aligned: installed.is_none_or(|value| value == expected),
    }
}

/// Read an installed `lqosd`'s protocol by running it with the probe flag
/// (`lqosd --print-protocol-version`) and assess alignment.
pub fn probe_installed(lqosd: &Path) -> SeamReport {
    let installed = Command::new(lqosd)
        .arg("--print-protocol-version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u32>().ok());
    assess(installed, EXPECTED_PROTOCOL_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn matching_protocols_align() {
        let report = assess(Some(EXPECTED_PROTOCOL_VERSION), EXPECTED_PROTOCOL_VERSION);
        assert!(report.aligned);
    }

    #[test]
    fn a_skewed_protocol_is_misaligned() {
        let old = EXPECTED_PROTOCOL_VERSION.wrapping_sub(1);
        let report = assess(Some(old), EXPECTED_PROTOCOL_VERSION);
        assert!(!report.aligned);
        assert_eq!(report.installed, Some(old));
        assert_eq!(report.expected, EXPECTED_PROTOCOL_VERSION);
    }

    #[test]
    fn an_absent_install_is_not_applicable() {
        assert!(assess(None, EXPECTED_PROTOCOL_VERSION).aligned);
    }

    #[test]
    fn probe_parses_a_scripted_lqosd() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lqos_seam_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("lqosd");
        // U1/W5: the scripted lqosd echoes THIS build's expected version — the
        // aligned case. (A protocol bump must not leave a stale literal here.)
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho {EXPECTED_PROTOCOL_VERSION}\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let report = probe_installed(&script);
        assert_eq!(report.installed, Some(EXPECTED_PROTOCOL_VERSION));
        assert!(report.aligned);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
