//! URL and tag validation shared by Agent and Server configuration.

use url::Url;

use super::{ConfigError, MAX_NODE_TAG_BYTES, MAX_NODE_TAGS};
use crate::validation::{normalize_string_list, validate_tag_list};

#[cfg(feature = "server-config")]
mod server;
#[cfg(feature = "server-config")]
pub use server::normalize_totp_secret;
#[cfg(feature = "server-config")]
pub(super) use server::{
    parse_trusted_proxies, uses_insecure_remote_public_base_url, validate_sha256,
    validate_totp_secret,
};

/// 校验 URL 字段:能被解析,并且采用了允许的协议方案。
pub(super) fn validate_url(field: &str, value: &str, schemes: &[&str]) -> Result<(), ConfigError> {
    let parsed =
        Url::parse(value).map_err(|error| ConfigError::new(format!("invalid {field}: {error}")))?;
    if !schemes.iter().any(|scheme| *scheme == parsed.scheme()) {
        return Err(ConfigError::new(format!(
            "{field} must use one of these schemes: {}",
            schemes.join(", ")
        )));
    }
    Ok(())
}

pub(super) fn normalize_tags(field: &str, values: Vec<String>) -> Result<Vec<String>, ConfigError> {
    let values = normalize_string_list(values);
    validate_tag_list(field, &values, MAX_NODE_TAGS, MAX_NODE_TAG_BYTES)?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validate_url_accepts_allowed_schemes_and_rejects_invalid_inputs() {
        validate_url(
            "server.public_base_url",
            "https://monitor.example.com",
            &["http", "https"],
        )
        .expect("https urls should pass");

        let error = validate_url("agent.server", "ftp://monitor.example.com", &["ws", "wss"])
            .expect_err("unexpected schemes should fail");
        assert_eq!(
            error.to_string(),
            "agent.server must use one of these schemes: ws, wss"
        );

        let error = validate_url("agent.server", "not a url", &["ws"])
            .expect_err("malformed urls should fail");
        assert!(error.to_string().contains("invalid agent.server"));
    }

    #[test]
    fn normalize_tags_trims_sorts_and_validates_lengths() {
        let tags = normalize_tags(
            "agent.tags",
            vec![" edge ".to_string(), "prod".to_string(), "edge".to_string()],
        )
        .expect("small tag lists should pass");
        assert_eq!(tags, vec!["edge".to_string(), "prod".to_string()]);

        let error = normalize_tags("agent.tags", vec!["a".repeat(257)])
            .expect_err("oversized tags should fail");
        assert_eq!(error.to_string(), "agent.tags[0] must be <= 256 bytes");
    }
}
