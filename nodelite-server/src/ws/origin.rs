//! 浏览器 WebSocket 握手的来源校验。
//!
//! 浏览器会为任意页面发起的 WebSocket 握手附带该主机已缓存的 Basic 凭据，而
//! WebSocket 不受 CORS 约束，所以 `/ws/browser` 不能只依赖凭据判断。`Origin`
//! 必须与本次请求的 `Host`（直连或保留 Host 的反向代理）或 `public_base_url`
//! 属于同一来源。没有 `Origin` 的请求不是浏览器发起的跨来源握手，保持放行，
//! 以兼容脚本与测试客户端。

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tracing::warn;
use url::Url;

/// Diagnostics never echo more than this many bytes of a client-supplied header.
const MAX_LOGGED_ORIGIN_BYTES: usize = 128;

pub(crate) fn browser_origin_allowed(headers: &HeaderMap, public_base_url: &str) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(origin) = origin.to_str().ok().and_then(parse_web_origin) else {
        return false;
    };
    host_header_url(headers, origin.scheme()).is_some_and(|host| host.origin() == origin.origin())
        || Url::parse(public_base_url).is_ok_and(|base| base.origin() == origin.origin())
}

pub(crate) fn cross_origin_rejected_response(headers: &HeaderMap) -> Response {
    let origin = headers
        .get(header::ORIGIN)
        .map(|value| String::from_utf8_lossy(value.as_bytes()));
    let origin = origin
        .as_deref()
        .map(|value| nodelite_proto::truncate_to_byte_boundary(value, MAX_LOGGED_ORIGIN_BYTES));
    warn!(origin = ?origin, "rejected browser websocket handshake from a foreign origin");
    StatusCode::FORBIDDEN.into_response()
}

fn parse_web_origin(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// Builds a URL from the request authority so it can be compared with `Origin` using the
/// same default-port normalization.
fn host_header_url(headers: &HeaderMap, scheme: &str) -> Option<Url> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let has_non_authority_chars = host
        .chars()
        .any(|c| matches!(c, '@' | '/' | '\\' | '?' | '#') || c.is_whitespace());
    if host.is_empty() || has_non_authority_chars {
        return None;
    }
    Url::parse(&format!("{scheme}://{host}")).ok()
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const BASE: &str = "https://monitor.example.com";

    fn headers(origin: Option<&str>, host: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(origin) = origin {
            headers.insert(
                header::ORIGIN,
                HeaderValue::from_str(origin).expect("origin header"),
            );
        }
        if let Some(host) = host {
            headers.insert(
                header::HOST,
                HeaderValue::from_str(host).expect("host header"),
            );
        }
        headers
    }

    #[test]
    fn handshake_without_origin_is_allowed() {
        assert!(browser_origin_allowed(
            &headers(None, Some("127.0.0.1:8080")),
            BASE
        ));
    }

    #[test]
    fn origin_matching_the_request_host_is_allowed() {
        let direct = headers(
            Some("http://192.168.1.100:8080"),
            Some("192.168.1.100:8080"),
        );
        assert!(browser_origin_allowed(&direct, "http://127.0.0.1:8080"));

        let proxied = headers(
            Some("https://Monitor.Example.com"),
            Some("monitor.example.com"),
        );
        assert!(browser_origin_allowed(&proxied, "http://127.0.0.1:8080"));

        let explicit_default_port = headers(
            Some("https://monitor.example.com"),
            Some("monitor.example.com:443"),
        );
        assert!(browser_origin_allowed(
            &explicit_default_port,
            "http://127.0.0.1:8080"
        ));

        let ipv6 = headers(Some("http://[::1]:8080"), Some("[::1]:8080"));
        assert!(browser_origin_allowed(&ipv6, BASE));
    }

    #[test]
    fn origin_matching_public_base_url_is_allowed_when_proxy_rewrites_host() {
        let rewritten = headers(Some("https://monitor.example.com"), Some("127.0.0.1:8080"));
        assert!(browser_origin_allowed(&rewritten, BASE));
    }

    #[test]
    fn foreign_origin_is_rejected() {
        let foreign = headers(
            Some("https://elsewhere.example"),
            Some("monitor.example.com"),
        );
        assert!(!browser_origin_allowed(&foreign, BASE));

        let other_port = headers(Some("http://127.0.0.1:9999"), Some("127.0.0.1:8080"));
        assert!(!browser_origin_allowed(
            &other_port,
            "http://127.0.0.1:8080"
        ));

        let other_scheme = headers(Some("http://monitor.example.com"), Some("127.0.0.1:8080"));
        assert!(!browser_origin_allowed(&other_scheme, BASE));
    }

    #[test]
    fn opaque_or_malformed_origins_are_rejected() {
        for origin in ["null", "file://", "chrome-extension://abcdef", "not a url"] {
            let request = headers(Some(origin), Some("monitor.example.com"));
            assert!(
                !browser_origin_allowed(&request, BASE),
                "{origin} should be rejected"
            );
        }
    }

    #[test]
    fn malformed_host_header_does_not_match() {
        let userinfo = headers(
            Some("https://evil.example"),
            Some("monitor.example.com@evil.example"),
        );
        assert!(!browser_origin_allowed(&userinfo, BASE));

        let missing_host = headers(Some("https://evil.example"), None);
        assert!(!browser_origin_allowed(&missing_host, BASE));
    }
}
