mod common;

use serial_test::serial;
use utoipa::OpenApi;

/// The committed document, resolved from the crate root (never the CWD).
fn committed_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json")
}

/// The pretty document exactly as it is committed: pretty JSON plus one
/// trailing newline.
fn pretty_document() -> String {
    let mut document = serde_json::to_string_pretty(&lqos_api::ApiDoc::openapi())
        .expect("serialize generated openapi to json");
    document.push('\n');
    document
}

const REGEN_HINT: &str = "Regenerate it with: cargo test -p lqos_api --test openapi_drift -- --ignored regen_openapi_json";

#[test]
#[serial]
fn openapi_json_matches_generated() {
    let _env = common::env();

    let generated = lqos_api::ApiDoc::openapi();
    let committed_raw =
        std::fs::read_to_string(committed_path()).expect("read committed openapi.json");

    // Byte equality, not `Value == Value`: property order is part of the
    // committed contract, so a re-serialization that reorders keys must fail.
    let expected = if committed_raw.ends_with('\n') {
        pretty_document()
    } else {
        pretty_document().trim_end_matches('\n').to_string()
    };
    assert_eq!(
        committed_raw, expected,
        "Committed openapi.json differs from ApiDoc::openapi() output. {REGEN_HINT}"
    );

    // The licence is a project constraint, not decoration: the API is
    // GPL-2.0-only project source (see `lqos_api/Cargo.toml`).
    let document: serde_json::Value =
        serde_json::from_str(&committed_raw).expect("parse committed openapi.json");
    let info = &document["info"];
    assert_eq!(
        info["license"]["name"], "GPL-2.0-only",
        "the OpenAPI licence must stay GPL-2.0-only"
    );
    let license_url = info["license"]["url"]
        .as_str()
        .expect("the OpenAPI licence must carry a URL");
    assert!(
        !license_url.trim().is_empty(),
        "the OpenAPI licence URL must not be empty"
    );
    let description = info["description"].as_str().unwrap_or_default();
    assert!(
        !description.contains("All Rights Reserved"),
        "the API description must not carry a proprietary licence claim"
    );
}

/// Writes the pretty document to the committed `openapi.json`.
///
/// Ignored by default: a normal run must never rewrite the contract it checks.
#[test]
#[ignore = "regenerates the committed openapi.json; run explicitly with --ignored"]
#[serial]
fn regen_openapi_json() {
    let _env = common::env();
    std::fs::write(committed_path(), pretty_document()).expect("write openapi.json");
}
