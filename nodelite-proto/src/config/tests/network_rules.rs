//! Network-facing server fields: trusted proxy ranges and the public base URL.

use super::super::parse_server_config;
use super::range_rules::{SERVER_SAMPLE, override_line, server_error};

#[test]
fn rejects_catch_all_trusted_proxies() {
    for value in [r#"["0.0.0.0/0"]"#, r#"["10.0.0.0/8", "::/0"]"#] {
        let error = server_error("trusted_proxies", value);
        assert!(error.contains("server.trusted_proxies"), "{error}");
    }
}

#[test]
fn rejects_public_base_url_with_path_query_or_fragment() {
    for value in [
        r#""https://monitor.example.com/nodelite""#,
        r#""https://monitor.example.com/nodelite/""#,
        r#""https://monitor.example.com/?a=1""#,
        r#""https://monitor.example.com/#top""#,
    ] {
        let error = server_error("public_base_url", value);
        assert!(error.contains("server.public_base_url"), "{value}: {error}");
    }
}

#[test]
fn accepts_public_base_url_origin_with_or_without_trailing_slash() {
    for value in [
        r#""https://monitor.example.com""#,
        r#""https://monitor.example.com/""#,
        r#""https://monitor.example.com:8443""#,
    ] {
        parse_server_config(&override_line(SERVER_SAMPLE, "public_base_url", value))
            .expect("origin-only public_base_url should parse");
    }
}
