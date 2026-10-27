//! Phase C mode-flag retirement gate (the ratchet).
//!
//! The integration-ingress "mode flag" — the canonical
//! `lqos_config::integration_ingress_enabled` predicate, its
//! `lqos_topology::flat_mode` and `lqos_python` copies, and the Python binding
//! `topology_import_ingress_enabled` — is being retired by Phase C. Once the
//! manual pair flows through the shared ingest payload (the dormant seam
//! `write_compiled_topology_from_legacy_artifacts` /
//! `write_compiled_topology_from_network_json_payload`), the payload's SOURCE
//! is not a mode and every branch on it collapses
//! (`docs/unified_simplification_plan.md` "THE MODE FLAG DIES";
//! `docs/sources_consolidation_card.md` §3 move 4).
//!
//! This workspace scan is the plan's "grep-gate oracle". While the flag is
//! retired, the tokens may appear ONLY in the enumerated allowlist:
//!   * a NEW file referencing the flag fails (no fourth copy, no new branch);
//!   * an allowlist entry whose file no longer references the flag must be
//!     REMOVED — the ratchet only tightens.
//! When the list is empty the flag is dead and the retirement gate is met.
//!
//! Self-excluded: this file names the tokens.

use std::path::{Path, PathBuf};

/// The two substrings that cover every predicate copy and the Python binding.
/// (`config_uses_topology_import_ingress` and
/// `topology_import_ingress_enabled` both contain `topology_import_ingress`.)
const MODE_FLAG_TOKENS: &[&str] = &["integration_ingress_enabled", "topology_import_ingress"];

/// Repo-root-relative files permitted to reference the mode flag while it is
/// retired. Shrinks monotonically; empty == the flag is dead.
///
/// Phase C S4 has emptied it: the predicate, its copies, the Python binding,
/// and every consumer are gone. THE MODE FLAG IS DEAD.
const MODE_FLAG_ALLOWLIST: &[&str] = &[];

fn collect(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            collect(&path, extension, out);
        } else if path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
}

fn contains_token(contents: &str) -> bool {
    MODE_FLAG_TOKENS.iter().any(|token| contents.contains(token))
}

#[test]
fn mode_flag_references_stay_within_the_retirement_allowlist() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .expect("lqos_config manifest should have a repo-root ancestor")
        .to_path_buf();
    let gate_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/mode_flag_retirement_gate.rs")
        .canonicalize()
        .expect("gate file should canonicalize");

    let mut files = Vec::new();
    collect(&repo_root.join("src/src/rust"), "rs", &mut files);
    collect(&repo_root.join("src/src"), "py", &mut files);
    assert!(
        files.len() > 100,
        "mode-flag scan should find many sources, found {}",
        files.len()
    );

    // (1) No file outside the allowlist may reference the flag.
    let mut violations = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for path in &files {
        if path.canonicalize().ok().as_deref() == Some(gate_path.as_path()) {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        if !contains_token(&contents) {
            continue;
        }
        let rel = path
            .strip_prefix(&repo_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        seen.push(rel.clone());
        if !MODE_FLAG_ALLOWLIST.contains(&rel.as_str()) {
            violations.push(format!("NEW mode-flag reference (add to the plan? no): {rel}"));
        }
    }

    // (2) The ratchet only tightens: an allowlist entry must still be needed.
    for allowed in MODE_FLAG_ALLOWLIST {
        if !seen.iter().any(|rel| rel == allowed) {
            violations.push(format!(
                "STALE allowlist entry (no longer references the flag — remove it): {allowed}"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Phase C mode-flag retirement gate failed (the integration-ingress mode flag is being \
         retired; it may appear only in the enumerated allowlist, and that list only shrinks):\n{}",
        violations.join("\n")
    );
}
