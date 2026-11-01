//! Network-facing server fields: trusted proxy ranges and the public base URL.

use super::range_rules::server_error;

#[test]
fn rejects_catch_all_trusted_proxies() {
    for value in [r#"["0.0.0.0/0"]"#, r#"["10.0.0.0/8", "::/0"]"#] {
        let error = server_error("trusted_proxies", value);
        assert!(error.contains("server.trusted_proxies"), "{error}");
    }
}
