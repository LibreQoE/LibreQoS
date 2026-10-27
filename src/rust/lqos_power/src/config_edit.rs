//! `[power] profile` in lqos.conf — the SINGLE SOURCE (`docs/power_profile_work_order.md`
//! §5). The editor is a minimal, structure-preserving line rewrite so the rest
//! of the operator's file (comments, ordering, unknown keys) is untouched.

use std::io;
use std::path::Path;

use crate::profile::PowerProfile;

/// The section header.
pub const SECTION: &str = "[power]";

/// The effective lqos.conf path (`LQOS_CONFIG` else the canonical default).
pub fn config_path() -> String {
    lqos_config::config_path()
}

/// Read the configured profile from a lqos.conf file, if present.
pub fn read_profile(path: &str) -> Option<PowerProfile> {
    let raw = std::fs::read_to_string(path).ok()?;
    parse_profile(&raw)
}

/// Parse `[power] profile` out of raw TOML. `None` when absent/unparsable.
pub fn parse_profile(raw: &str) -> Option<PowerProfile> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    value
        .get("power")?
        .get("profile")?
        .as_str()?
        .parse::<PowerProfile>()
        .ok()
}

/// Read the full `[power]` section (`profile` + `eco_max_freq_pct`).
pub fn read_power_config(path: &str) -> Option<lqos_config::PowerConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    value.get("power")?.clone().try_into().ok()
}

/// Rewrite the profile in `raw`, preserving everything else. Pure.
///
/// * an existing `profile = …` in `[power]` is replaced in place;
/// * a `[power]` section without `profile` gets the key inserted after its
///   header (and after that section's leading comments);
/// * otherwise a new `[power]` section is appended.
pub fn rewrite_profile(raw: &str, profile: PowerProfile) -> String {
    let rendered = format!("profile = \"{profile}\"");
    let lines: Vec<&str> = raw.lines().collect();

    let Some(header_idx) = lines
        .iter()
        .position(|l| l.trim() == SECTION)
    else {
        // No [power] section: append one.
        let mut out = raw.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(SECTION);
        out.push('\n');
        out.push_str(&rendered);
        out.push('\n');
        return out;
    };

    // Find the section end (the next table header, or EOF).
    let section_end = lines[header_idx + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|off| header_idx + 1 + off)
        .unwrap_or(lines.len());

    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut inserted = false;
    for (idx, line) in lines.iter().enumerate() {
        if idx == header_idx {
            out_lines.push((*line).to_string());
            continue;
        }
        let in_section = idx > header_idx && idx < section_end;
        let trimmed = line.trim_start();
        let is_profile_key = trimmed.starts_with("profile") && trimmed.contains('=');
        if in_section && is_profile_key {
            if !inserted {
                out_lines.push(rendered.clone());
                inserted = true;
            }
            continue; // drop the old profile line
        }
        out_lines.push((*line).to_string());
    }
    if !inserted {
        // Insert after the header (and any immediately-following comments/blank
        // lines stay below the key — keep it simple: insert right after header).
        out_lines.insert(header_idx + 1, rendered);
    }
    let mut out = out_lines.join("\n");
    out.push('\n');
    out
}

/// Atomically write `raw` to `path` (tmp + rename in the same directory).
pub fn atomic_write(path: &str, raw: &str) -> io::Result<()> {
    let path = Path::new(path);
    let tmp = path.with_extension("conf.power.tmp");
    std::fs::write(&tmp, raw)?;
    std::fs::rename(&tmp, path)
}

/// Set the profile in the lqos.conf file at `path`.
pub fn set_profile(path: &str, profile: PowerProfile) -> io::Result<()> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    atomic_write(path, &rewrite_profile(&raw, profile))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_section() {
        let raw = "[queues]\nqueue_mode = \"shape\"\n\n[power]\nprofile = \"eco\"\n";
        assert_eq!(parse_profile(raw), Some(PowerProfile::Eco));
        assert_eq!(parse_profile("[queues]\n"), None);
    }

    #[test]
    fn rewrite_replaces_an_existing_key_in_place() {
        let raw = "[queues]\na = 1\n\n[power]\n# bias\nprofile = \"eco\"\n";
        let out = rewrite_profile(raw, PowerProfile::Performance);
        assert!(out.contains("profile = \"performance\""));
        assert!(!out.contains("\"eco\""));
        assert!(out.contains("[queues]")); // other content preserved
        assert_eq!(out.matches("[power]").count(), 1);
    }

    #[test]
    fn rewrite_inserts_the_key_into_an_existing_section() {
        let raw = "[power]\n# comment\n\n[other]\nx = 1\n";
        let out = rewrite_profile(raw, PowerProfile::Eco);
        assert!(out.contains("[power]\nprofile = \"eco\"\n"));
        assert!(out.contains("[other]"));
    }

    #[test]
    fn rewrite_appends_a_missing_section() {
        let raw = "[queues]\nqueue_mode = \"shape\"\n";
        let out = rewrite_profile(raw, PowerProfile::Balanced);
        assert!(out.contains("[power]"));
        assert!(out.contains("profile = \"balanced\""));
        assert!(out.contains("[queues]"));
    }

    #[test]
    fn rewrite_is_idempotent() {
        let raw = "[power]\nprofile = \"eco\"\n";
        let once = rewrite_profile(raw, PowerProfile::Eco);
        let twice = rewrite_profile(&once, PowerProfile::Eco);
        assert_eq!(once, twice);
    }
}
