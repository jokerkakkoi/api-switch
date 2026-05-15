use axum::http::{HeaderMap, HeaderName, HeaderValue};

const SKIP_HEADERS: &[&str] = &[
    "authorization",
    "content-length",
    "host",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "te",
    "trailers",
    "upgrade",
];

/// Build forwarded headers from incoming headers, skipping hop-by-hop headers
/// and adding `app-key` and `app-sign` from config.
pub fn build_forwarded_headers(incoming: &HeaderMap, app_key: &str, app_sign: &str) -> HeaderMap {
    let mut fwd = HeaderMap::new();
    for (key, value) in incoming.iter() {
        if !SKIP_HEADERS.contains(&key.as_str()) {
            fwd.insert(key, value.clone());
        }
    }
    fwd.insert(
        HeaderName::from_static("app-key"),
        HeaderValue::from_str(app_key).expect("invalid app_key"),
    );
    fwd.insert(
        HeaderName::from_static("app-sign"),
        HeaderValue::from_str(app_sign).expect("invalid app_sign"),
    );
    fwd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filters_authorization_header() {
        let mut incoming = HeaderMap::new();
        incoming.insert("authorization", HeaderValue::from_static("Bearer token"));

        let result = build_forwarded_headers(&incoming, "key", "sign");

        assert!(result.get("authorization").is_none());
    }

    #[test]
    fn test_filters_all_skip_headers() {
        let mut incoming = HeaderMap::new();
        incoming.insert("authorization", HeaderValue::from_static("Bearer token"));
        incoming.insert("content-length", HeaderValue::from_static("100"));
        incoming.insert("content-type", HeaderValue::from_static("application/json"));
        incoming.insert("host", HeaderValue::from_static("example.com"));
        incoming.insert("transfer-encoding", HeaderValue::from_static("chunked"));
        incoming.insert("connection", HeaderValue::from_static("keep-alive"));
        incoming.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        incoming.insert("te", HeaderValue::from_static("trailers"));
        incoming.insert("trailers", HeaderValue::from_static("X-Trailers"));
        incoming.insert("upgrade", HeaderValue::from_static("websocket"));

        let result = build_forwarded_headers(&incoming, "key", "sign");

        for header in SKIP_HEADERS {
            assert!(
                result.get(*header).is_none(),
                "header '{}' should be filtered",
                header
            );
        }
    }

    #[test]
    fn test_preserves_non_skip_headers() {
        let mut incoming = HeaderMap::new();
        incoming.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        incoming.insert("x-custom", HeaderValue::from_static("custom-value"));

        let result = build_forwarded_headers(&incoming, "key", "sign");

        assert_eq!(
            result.get("anthropic-version").unwrap().to_str().unwrap(),
            "2023-06-01"
        );
        assert_eq!(
            result.get("x-custom").unwrap().to_str().unwrap(),
            "custom-value"
        );
    }

    #[test]
    fn test_adds_app_key_and_app_sign() {
        let incoming = HeaderMap::new();
        let result = build_forwarded_headers(&incoming, "my-key", "my-sign");

        assert_eq!(result.get("app-key").unwrap().to_str().unwrap(), "my-key");
        assert_eq!(result.get("app-sign").unwrap().to_str().unwrap(), "my-sign");
    }

    #[test]
    fn test_empty_incoming_headers() {
        let incoming = HeaderMap::new();
        let result = build_forwarded_headers(&incoming, "key", "sign");

        assert_eq!(result.len(), 2); // only app-key and app-sign
        assert_eq!(result.get("app-key").unwrap().to_str().unwrap(), "key");
        assert_eq!(result.get("app-sign").unwrap().to_str().unwrap(), "sign");
    }
}
