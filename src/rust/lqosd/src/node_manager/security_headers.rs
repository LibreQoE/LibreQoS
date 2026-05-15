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

pub(crate) fn apply_node_manager_security_headers(headers: &mut HeaderMap) {
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(NODE_MANAGER_CONTENT_SECURITY_POLICY),
    );
}

#[cfg(test)]
mod tests {
    use super::{NODE_MANAGER_CONTENT_SECURITY_POLICY, apply_node_manager_security_headers};
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

        apply_node_manager_security_headers(&mut headers);

        let header_name = HeaderName::from_static("content-security-policy");
        assert_eq!(
            headers.get(header_name),
            Some(&NODE_MANAGER_CONTENT_SECURITY_POLICY.parse().unwrap())
        );
    }
}
