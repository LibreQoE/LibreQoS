//! Request-scoped UI locale for the node manager: the compiled-in catalog
//! registry, `Accept-Language` negotiation, and the `Content-Language` rule the
//! page-serving paths share.
//!
//! The registry is embedded (`include_str!`) so this server can only ever offer
//! a locale whose catalog the same build produced.

use axum::http::header;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum_extra::extract::CookieJar;
use once_cell::sync::Lazy;
use std::sync::Once;
use tracing::{error, warn};

/// Cookie the browser writes when someone picks a UI language.
///
/// The switcher writes the same name from JavaScript
/// (`js_build/src/i18n/index.js`); neither side can import the other, so
/// `test-build-contract.sh` asserts the two literals still agree.
const UI_LANGUAGE_COOKIE: &str = "lqos_lang";

/// The locale served when nothing else matches. Its catalog is also the fallback
/// every page loads.
pub(crate) const DEFAULT_UI_LOCALE: &str = "en";

/// The catalog list shipped beside these assets.
const LOCALES_JSON: &str = include_str!("static2/i18n/locales.json");

#[derive(serde::Deserialize)]
struct LocaleRecord {
    tag: String,
    #[serde(default)]
    dir: Option<String>,
}

/// One locale this build can serve: the tag a request resolves to, the primary
/// subtag matching uses, and the text direction from `locales.json`.
pub(crate) struct UiLocale {
    tag: String,
    primary: String,
    dir: &'static str,
}

static UI_LOCALES: Lazy<Vec<UiLocale>> = Lazy::new(|| parse_locales(LOCALES_JSON));

/// Set once the operator default has been reported as unknown.
static UNKNOWN_CONFIG_LANGUAGE_WARNED: Once = Once::new();

/// Parses a locale list, dropping any record whose tag could not be echoed into
/// an HTTP header or an HTML attribute.
///
/// This is the single validation point for every served tag: the page paths
/// write `Content-Language` and `lang=` without re-checking. A dropped record
/// degrades that locale to English instead of failing a request, so a bad entry
/// in a compiled-in asset never turns into a 500.
pub(crate) fn parse_locales(raw: &str) -> Vec<UiLocale> {
    let Ok(records) = serde_json::from_str::<Vec<LocaleRecord>>(raw) else {
        error!("Cannot parse the bundled locale list; no UI locale is selectable");
        return Vec::new();
    };
    records
        .into_iter()
        .filter_map(|record| {
            if !is_servable_tag(&record.tag) {
                error!(
                    "Dropping locale {:?} from the bundled locale list: not a header-safe language tag",
                    record.tag
                );
                return None;
            }
            Some(UiLocale {
                primary: primary_subtag(&record.tag),
                dir: if record.dir.as_deref() == Some("rtl") {
                    "rtl"
                } else {
                    "ltr"
                },
                tag: record.tag,
            })
        })
        .collect()
}

/// Whether a tag can be served: ASCII letters and digits plus the `-`/`_`
/// separators locale tags use.
///
/// That set is a subset of both an HTTP field value and an HTML attribute value,
/// so an accepted tag is safe in `Content-Language`, in `lang=`, and in the
/// catalog URL this build asks the browser to load.
fn is_servable_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

/// The lowercased primary subtag of a locale tag (`es-MX` and `es_MX` both give
/// `es`). Empty input gives an empty string.
fn primary_subtag(tag: &str) -> String {
    tag.trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The tag in `locales` matching `candidate`, case-insensitively and by primary
/// subtag, or `None` when the list has no catalog for it.
fn lookup_locale_in<'a>(locales: &'a [UiLocale], candidate: &str) -> Option<&'a str> {
    let lowered = candidate.trim().to_ascii_lowercase();
    if lowered.is_empty() || lowered == "*" {
        return None;
    }
    if let Some(exact) = locales
        .iter()
        .find(|locale| locale.tag.eq_ignore_ascii_case(&lowered))
    {
        return Some(exact.tag.as_str());
    }
    let primary = primary_subtag(&lowered);
    locales
        .iter()
        .find(|locale| locale.primary == primary)
        .map(|locale| locale.tag.as_str())
}

/// [`lookup_locale_in`] against the list this build ships.
fn lookup_locale(candidate: &str) -> Option<&'static str> {
    lookup_locale_in(&UI_LOCALES, candidate)
}

/// Text direction for `tag` in `locales`, from the record's metadata. An unknown
/// tag reports `ltr`.
///
/// Takes the list explicitly so the direction path can be exercised over a list
/// this build does not ship (no `rtl` catalog is bundled today).
pub(crate) fn dir_for_in(locales: &[UiLocale], tag: &str) -> &'static str {
    locales
        .iter()
        .find(|locale| locale.tag == tag)
        .map_or("ltr", |locale| locale.dir)
}

/// [`dir_for_in`] against the list this build ships.
pub(crate) fn dir_for(tag: &str) -> &'static str {
    dir_for_in(&UI_LOCALES, tag)
}

/// Tags from an `Accept-Language` field value, highest q-value first and header
/// order for ties. `*`, `q=0` and entries with an unparsable q-value are
/// dropped.
fn accept_language_candidates(header: &str) -> Vec<&str> {
    let mut scored: Vec<(f32, &str)> = Vec::new();
    for entry in header.split(',') {
        let mut parts = entry.split(';');
        let tag = parts.next().unwrap_or_default().trim();
        if tag.is_empty() || tag == "*" {
            continue;
        }
        let mut quality = 1.0_f32;
        let mut malformed = false;
        for parameter in parts {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            if !name.trim().eq_ignore_ascii_case("q") {
                continue;
            }
            match value.trim().parse::<f32>() {
                Ok(parsed) if (0.0..=1.0).contains(&parsed) => quality = parsed,
                _ => malformed = true,
            }
        }
        if malformed || quality <= 0.0 {
            continue;
        }
        scored.push((quality, tag));
    }
    // `sort_by` is stable, so equal q-values keep the header's order.
    scored.sort_by(|left, right| right.0.total_cmp(&left.0));
    scored.into_iter().map(|(_, tag)| tag).collect()
}

/// Every `Accept-Language` field line of `headers`, comma-joined.
///
/// RFC 9110 §5.3 lets a sender split one list field across lines, so reading only
/// the first line would silently drop the rest of the caller's preference.
/// Field lines that are not text cannot be a language list and are skipped.
fn accept_language_header(headers: &HeaderMap) -> Option<String> {
    let mut joined = String::new();
    for value in headers.get_all(header::ACCEPT_LANGUAGE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        if !joined.is_empty() {
            joined.push(',');
        }
        joined.push_str(value);
    }
    (!joined.is_empty()).then_some(joined)
}

/// Resolves the UI locale for one request: the browser's cookie, then the
/// node-wide `ui_language`, then `Accept-Language`, then English.
///
/// Only tags this build ships a catalog for are ever returned; a tag without a
/// catalog is treated as absent at every step, so `de` or `en-XA` resolve to
/// `en`.
pub(crate) fn resolve_ui_locale(
    cookie_lang: Option<&str>,
    config_lang: Option<&str>,
    accept_language: Option<&str>,
) -> String {
    if let Some(tag) = cookie_lang.and_then(lookup_locale) {
        return tag.to_string();
    }
    if let Some(tag) = config_lang.and_then(lookup_locale) {
        return tag.to_string();
    }
    if let Some(header) = accept_language {
        for candidate in accept_language_candidates(header) {
            if let Some(tag) = lookup_locale(candidate) {
                return tag.to_string();
            }
        }
    }
    DEFAULT_UI_LOCALE.to_string()
}

/// [`resolve_ui_locale`] for a live request, warning once per process when the
/// configured `ui_language` names no catalog this build ships.
///
/// Side effects: logs at most one warning.
pub(crate) fn request_ui_locale(
    config_lang: Option<&str>,
    jar: &CookieJar,
    headers: &HeaderMap,
) -> String {
    if let Some(tag) = config_lang
        && lookup_locale(tag).is_none()
    {
        UNKNOWN_CONFIG_LANGUAGE_WARNED.call_once(|| {
            warn!(
                "ui_language {tag:?} has no shipped catalog; using the browser or Accept-Language choice instead"
            );
        });
    }
    let accept_language = accept_language_header(headers);
    resolve_ui_locale(
        jar.get(UI_LANGUAGE_COOKIE).map(|cookie| cookie.value()),
        config_lang,
        accept_language.as_deref(),
    )
}

/// Writes `Content-Language: tag`.
///
/// Every page-serving path calls this, so the header cannot name the locale on
/// one page kind and be missing from another. `parse_locales` already dropped
/// tags a header cannot carry, so the error arm only fires for a tag that never
/// came from the registry — it is reported, never assumed away.
pub(crate) fn set_content_language(headers: &mut HeaderMap, tag: &str) -> Result<(), StatusCode> {
    let value = HeaderValue::from_str(tag).map_err(|_| {
        error!("UI locale {tag:?} cannot be sent as Content-Language");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    headers.insert(header::CONTENT_LANGUAGE, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        UI_LOCALES, accept_language_header, dir_for, dir_for_in, parse_locales, resolve_ui_locale,
    };
    use axum::http::header;
    use axum::http::{HeaderMap, HeaderValue};

    /// A locale list this build does not ship: the right-to-left path has no
    /// shipped catalog today, so the records themselves have to carry it.
    const RTL_LOCALES: &str = r#"[{"tag":"en","dir":"ltr"},{"tag":"he","dir":"rtl"}]"#;

    #[test]
    fn the_shipped_catalog_list_offers_english_and_no_pseudo_locale() {
        assert!(UI_LOCALES.iter().any(|locale| locale.tag == "en"));
        assert!(
            !UI_LOCALES.iter().any(|locale| locale.tag == "en-XA"),
            "en-XA is built for pseudo-localization, never offered"
        );
    }

    #[test]
    fn direction_comes_from_the_catalog_metadata() {
        assert_eq!(dir_for("en"), "ltr");
        assert_eq!(dir_for("es"), "ltr");
        assert_eq!(dir_for("de"), "ltr", "an unknown tag reports the default");
        // The shipped list has no rtl catalog, so the record is where the
        // direction path is provable.
        let locales = parse_locales(RTL_LOCALES);
        assert_eq!(dir_for_in(&locales, "he"), "rtl");
        assert_eq!(dir_for_in(&locales, "en"), "ltr");
        assert_eq!(
            dir_for_in(&locales, "he-IL"),
            "ltr",
            "only exact tags carry"
        );
    }

    #[test]
    fn a_locale_tag_that_cannot_be_sent_as_a_header_is_dropped() {
        let locales =
            parse_locales(r#"[{"tag":"en","dir":"ltr"},{"tag":"es-Ñ"},{"tag":"fr"},{"tag":""}]"#);
        let tags: Vec<&str> = locales.iter().map(|locale| locale.tag.as_str()).collect();
        assert_eq!(
            tags,
            ["en", "fr"],
            "a tag a header cannot carry must not reach the page paths"
        );
        for tag in tags {
            assert!(
                HeaderValue::from_str(tag).is_ok(),
                "{tag} must be header-safe once the registry accepted it"
            );
        }
    }

    #[test]
    fn cookie_wins_over_config_and_accept_language() {
        assert_eq!(
            resolve_ui_locale(Some("es"), Some("en"), Some("en;q=0.9")),
            "es"
        );
    }

    #[test]
    fn config_wins_over_accept_language() {
        assert_eq!(resolve_ui_locale(None, Some("es"), Some("en;q=0.9")), "es");
    }

    #[test]
    fn accept_language_is_the_last_choice_before_english() {
        assert_eq!(
            resolve_ui_locale(None, None, Some("es-MX,es;q=0.9,en;q=0.8")),
            "es"
        );
    }

    #[test]
    fn english_is_the_default() {
        assert_eq!(resolve_ui_locale(None, None, None), "en");
        assert_eq!(resolve_ui_locale(Some(""), Some("  "), Some("")), "en");
    }

    #[test]
    fn matching_is_case_insensitive_and_ignores_the_region() {
        assert_eq!(resolve_ui_locale(Some("ES"), None, None), "es");
        assert_eq!(resolve_ui_locale(Some("es_MX"), None, None), "es");
        assert_eq!(resolve_ui_locale(None, Some("ES-mx"), None), "es");
    }

    #[test]
    fn accept_language_orders_by_quality_keeping_header_order_for_ties() {
        assert_eq!(
            resolve_ui_locale(None, None, Some("de;q=0.9,es;q=0.2")),
            "es",
            "an unmatched tag never outranks a catalog tag"
        );
        assert_eq!(
            resolve_ui_locale(None, None, Some("es;q=0.2,en;q=0.9")),
            "en",
            "the higher q-value wins even when it comes later in the header"
        );
        assert_eq!(
            resolve_ui_locale(None, None, Some("es;q=0.5,en;q=0.5")),
            "es",
            "equal q-values keep the header order"
        );
        assert_eq!(
            resolve_ui_locale(None, None, Some("en;q=0.9,es;q=0.9")),
            "en",
            "equal q-values keep the header order"
        );
    }

    #[test]
    fn accept_language_skips_wildcards_excluded_and_malformed_entries() {
        assert_eq!(resolve_ui_locale(None, None, Some("*;q=0.9,en;q=0")), "en");
        assert_eq!(resolve_ui_locale(None, None, Some("es;q=abc,en")), "en");
        assert_eq!(resolve_ui_locale(None, None, Some("es-MX;q=1.5,en")), "en");
        assert_eq!(resolve_ui_locale(None, None, Some(";;;, ,")), "en");
        assert_eq!(resolve_ui_locale(None, None, Some("es;level=1")), "es");
    }

    #[test]
    fn an_excluded_language_is_never_a_candidate() {
        // `q=0` means "not acceptable", so the only remaining choice is the
        // default. This separates the drop from the wildcard case above, which
        // resolves to English with or without it.
        assert_eq!(resolve_ui_locale(None, None, Some("es;q=0")), "en");
    }

    #[test]
    fn accept_language_field_lines_are_joined() {
        let mut headers = HeaderMap::new();
        headers.append(header::ACCEPT_LANGUAGE, HeaderValue::from_static("de"));
        headers.append(header::ACCEPT_LANGUAGE, HeaderValue::from_static("es"));

        let joined = accept_language_header(&headers).expect("two field lines");
        assert_eq!(joined, "de,es", "both field lines make up one list");
        assert_eq!(
            resolve_ui_locale(None, None, Some(&joined)),
            "es",
            "the second line must not be lost"
        );
        assert_eq!(accept_language_header(&HeaderMap::new()), None);
    }

    #[test]
    fn tags_without_a_catalog_fall_back_to_english() {
        for tag in ["de", "zz", "en-XA", "fr-CA"] {
            assert_eq!(resolve_ui_locale(Some(tag), None, None), "en", "{tag}");
        }
        assert_eq!(resolve_ui_locale(None, Some("de"), None), "en");
        assert_eq!(
            resolve_ui_locale(Some("es-419"), None, None),
            "es",
            "a regional variant resolves to its primary catalog"
        );
    }
}
