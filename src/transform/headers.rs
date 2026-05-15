use axum::http::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashSet;

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

/// Parse the Connection header value to extract declared hop-by-hop header names per RFC 7230 Section 6.1.
fn parse_connection_tokens(connection_value: &str) -> HashSet<String> {
    connection_value
        .split(',')
        .map(|token| token.trim().to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

/// Build forwarded headers from incoming headers, skipping hop-by-hop headers
/// (both standard ones and those declared in the Connection header per RFC 7230 Section 6.1)
/// and adding `app-key` and `app-sign` from config.
pub fn build_forwarded_headers(incoming: &HeaderMap, app_key: &str, app_sign: &str) -> HeaderMap {
    let mut skip_set: HashSet<&str> = SKIP_HEADERS.iter().copied().collect();

    // Parse Connection header for dynamic hop-by-hop declarations
    if let Some(conn_value) = incoming.get("connection") {
        if let Ok(conn_str) = conn_value.to_str() {
            let dynamic_hops = parse_connection_tokens(conn_str);
            for hop in &dynamic_hops {
                skip_set.insert(Box::leak(hop.clone().into_boxed_str()));
            }
        }
    }

    let mut fwd = HeaderMap::new();
    for (key, value) in incoming.iter() {
        if !skip_set.contains(key.as_str()) {
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
    fn test_filters_connection_declared_hop_by_hop_headers() {
        let mut incoming = HeaderMap::new();
        incoming.insert("connection", HeaderValue::from_static("x-custom, x-another"));
        incoming.insert("x-custom", HeaderValue::from_static("should-be-filtered"));
        incoming.insert("x-another", HeaderValue::from_static("also-filtered"));
        incoming.insert("x-not-declared", HeaderValue::from_static("should-pass"));

        let result = build_forwarded_headers(&incoming, "key", "sign");

        assert!(result.get("x-custom").is_none(), "x-custom should be filtered");
        assert!(result.get("x-another").is_none(), "x-another should be filtered");
        assert!(result.get("x-not-declared").is_some(), "x-not-declared should pass");
    }

    #[test]
    fn test_connection_case_insensitive_and_whitespace() {
        let mut incoming = HeaderMap::new();
        incoming.insert("connection", HeaderValue::from_static("  X-Custom ,  KEEP-ALIVE  , te "));
        incoming.insert("x-custom", HeaderValue::from_static("filtered"));
        incoming.insert("keep-alive", HeaderValue::from_static("filtered"));
        incoming.insert("te", HeaderValue::from_static("filtered"));
        incoming.insert("x-other", HeaderValue::from_static("passes"));

        let result = build_forwarded_headers(&incoming, "key", "sign");

        assert!(result.get("x-custom").is_none());
        assert!(result.get("keep-alive").is_none());
        assert!(result.get("te").is_none());
        assert!(result.get("x-other").is_some());
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
