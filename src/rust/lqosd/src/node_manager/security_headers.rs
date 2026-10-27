use axum::http::{HeaderMap, HeaderName, HeaderValue};

// The current templates still use small inline boot scripts and inline layout styles.
// Keep script/style sources otherwise self-hosted so future remote script drift is visible.
pub(crate) const NODE_MANAGER_CONTENT_SECURITY_POLICY: &str = concat!(
    "default-src 'self'; ",
    "base-uri 'self'; ",
    "object-src 'none'; ",
    "form-action 'self'; ",
    "frame-ancestors 'none'; ",
    "script-src 'self' 'unsafe-inline'; ",
    "style-src 'self' 'unsafe-inline'; ",
    "img-src 'self' data: blob: https://insight.libreqos.com; ",
    "font-src 'self' data:; ",
    "connect-src 'self' ws: wss: https://insight.libreqos.com; ",
    "frame-src 'self' about: http://*:9122; ",
    "worker-src 'self' blob:"
);

/// The node-manager CSP, optionally allowing an EXTRA tile host (the configured
/// Insight base) in `img-src`/`connect-src` so a non-default Site Map tile
/// source is not blocked.  The default host is always allowed.
pub(crate) fn node_manager_content_security_policy(extra_host: Option<&str>) -> String {
    let extra = extra_host
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(|host| format!(" {host}"))
        .unwrap_or_default();
    NODE_MANAGER_CONTENT_SECURITY_POLICY
        .replace(
            "img-src 'self' data: blob: https://insight.libreqos.com;",
            &format!("img-src 'self' data: blob: https://insight.libreqos.com{extra};"),
        )
        .replace(
            "connect-src 'self' ws: wss: https://insight.libreqos.com;",
            &format!("connect-src 'self' ws: wss: https://insight.libreqos.com{extra};"),
        )
}

/// The scheme://authority origin of the configured Insight base, or `None` when
/// it is unset or the default (already in the static policy).
pub(crate) fn insight_csp_origin(lts_url: Option<&str>) -> Option<String> {
    let base = crate::node_manager::local_api::lts::normalize_insight_base_url(lts_url);
    let origin = match base.find("://") {
        Some(pos) => {
            let after = &base[pos + 3..];
            match after.find('/') {
                Some(slash) => base[..pos + 3 + slash].to_string(),
                None => base.clone(),
            }
        }
        None => base.trim_end_matches('/').to_string(),
    };
    if origin == "https://insight.libreqos.com" {
        None
    } else {
        Some(origin)
    }
}

pub(crate) fn apply_node_manager_security_headers(headers: &mut HeaderMap, extra_host: Option<&str>) {
    let csp = node_manager_content_security_policy(extra_host);
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(HeaderName::from_static("content-security-policy"), value);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NODE_MANAGER_CONTENT_SECURITY_POLICY, apply_node_manager_security_headers,
        insight_csp_origin, node_manager_content_security_policy,
    };
    use axum::http::{HeaderMap, HeaderName};

    #[test]
    fn node_manager_csp_matches_current_ui_requirements() {
        let csp = NODE_MANAGER_CONTENT_SECURITY_POLICY;

        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("script-src 'self' 'unsafe-inline'"));
        assert!(csp.contains("style-src 'self' 'unsafe-inline'"));
        assert!(csp.contains("connect-src 'self' ws: wss: https://insight.libreqos.com"));
        assert!(csp.contains("img-src 'self' data: blob: https://insight.libreqos.com"));
        assert!(csp.contains("frame-src 'self' about: http://*:9122"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains("https://fastly.jsdelivr.net"));
    }

    #[test]
    fn applies_content_security_policy_header() {
        let mut headers = HeaderMap::new();

        apply_node_manager_security_headers(&mut headers, None);

        let header_name = HeaderName::from_static("content-security-policy");
        assert_eq!(
            headers.get(header_name),
            Some(&NODE_MANAGER_CONTENT_SECURITY_POLICY.parse().unwrap())
        );
    }

    #[test]
    fn csp_allows_a_configured_tile_host() {
        let csp = node_manager_content_security_policy(Some("https://tiles.example"));
        assert!(csp.contains(
            "img-src 'self' data: blob: https://insight.libreqos.com https://tiles.example;"
        ));
        assert!(csp.contains(
            "connect-src 'self' ws: wss: https://insight.libreqos.com https://tiles.example;"
        ));

        assert_eq!(insight_csp_origin(None), None);
        assert_eq!(insight_csp_origin(Some("")), None);
        assert_eq!(insight_csp_origin(Some("insight.libreqos.com")), None);
        assert_eq!(
            insight_csp_origin(Some("https://tiles.example/base/")),
            Some("https://tiles.example".to_string())
        );
    }
}
