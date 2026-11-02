//! Embed the API build revision for `/version`.
//!
//! `LQOS_BUILD_REV` wins when it is a lower-case 12-hex short revision (release
//! builds). Otherwise try `git rev-parse --short=12 HEAD`, and fall back to
//! `unknown` when neither is available.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LQOS_BUILD_REV");
    println!("cargo:rerun-if-env-changed=LQOS_RELEASE_REV");

    let rev = std::env::var("LQOS_BUILD_REV")
        .ok()
        .filter(|value| is_hex12(value))
        .or_else(git_rev)
        .unwrap_or_else(|| "unknown".to_string());

    // Stale-artifact gate: a release build may pin the expected short rev.
    // A mismatch means the binary was built from the wrong tree.
    if let Ok(expected) = std::env::var("LQOS_RELEASE_REV")
        && is_hex12(&expected)
        && rev != expected
    {
        panic!("LQOS_BUILD_REV {rev} does not match LQOS_RELEASE_REV {expected}");
    }

    println!("cargo:rustc-env=LQOS_BUILD_REV={rev}");
}

fn git_rev() -> Option<String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(["-C", &manifest, "rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let rev = String::from_utf8(output.stdout).ok()?.trim().to_string();
    is_hex12(&rev).then_some(rev)
}

fn is_hex12(value: &str) -> bool {
    value.len() == 12
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
