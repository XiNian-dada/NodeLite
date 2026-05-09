use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

pub const DEFAULT_STALE_AFTER_SECS: u64 = 20;
pub const DEFAULT_PING_INTERVAL_SECS: u64 = 10;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 5;
pub const DEFAULT_REPORT_INTERVAL_SECS: u64 = 5;
pub const DEFAULT_HISTORY_RETENTION_HOURS: u64 = 72;
pub const DEFAULT_HISTORY_WRITE_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub public_base_url: String,
    pub readonly_auth: Option<ReadonlyAuthConfig>,
    pub node_registry_path: PathBuf,
    pub history_db_path: PathBuf,
    pub snapshot_path: PathBuf,
    pub stale_after_secs: u64,
    pub ping_interval_secs: u64,
    pub max_message_bytes: usize,
    pub refresh_interval_secs: u64,
    pub ignored_filesystems: Vec<String>,
    pub agent_release_base_url: Option<String>,
    pub agent_release_sha256_x86_64: Option<String>,
    pub agent_release_sha256_aarch64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadonlyAuthConfig {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    pub node_id: String,
    pub node_label: String,
    pub server: String,
    pub token: String,
    pub report_interval_secs: u64,
    pub hostname_override: Option<String>,
    pub tags: Vec<String>,
}

pub fn parse_server_config(input: &str) -> Result<ServerConfig, ConfigError> {
    let raw: RawServerConfigFile =
        toml::from_str(input).map_err(|error| ConfigError::new(error.to_string()))?;
    raw.validate()
}

pub fn parse_agent_config(input: &str) -> Result<AgentConfig, ConfigError> {
    let raw: RawAgentConfigFile =
        toml::from_str(input).map_err(|error| ConfigError::new(error.to_string()))?;
    raw.validate()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServerConfigFile {
    server: RawServerSection,
    #[serde(default)]
    auth: RawAuthSection,
    #[serde(default)]
    ui: RawUiSection,
    #[serde(default)]
    filters: RawFiltersSection,
    #[serde(default)]
    install: RawInstallSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServerSection {
    listen: String,
    public_base_url: String,
    #[serde(default = "default_node_registry_path")]
    node_registry_path: PathBuf,
    #[serde(default = "default_history_db_path")]
    history_db_path: PathBuf,
    #[serde(default = "default_snapshot_path")]
    snapshot_path: PathBuf,
    #[serde(default = "default_stale_after_secs")]
    stale_after_secs: u64,
    #[serde(default = "default_ping_interval_secs")]
    ping_interval_secs: u64,
    #[serde(default = "default_max_message_bytes")]
    max_message_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUiSection {
    #[serde(default = "default_refresh_interval_secs")]
    refresh_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAuthSection {
    username: Option<String>,
    password: Option<String>,
}

impl Default for RawUiSection {
    fn default() -> Self {
        Self {
            refresh_interval_secs: default_refresh_interval_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFiltersSection {
    #[serde(default = "default_ignored_filesystems")]
    ignored_filesystems: Vec<String>,
}

impl Default for RawFiltersSection {
    fn default() -> Self {
        Self {
            ignored_filesystems: default_ignored_filesystems(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawInstallSection {
    agent_release_base_url: Option<String>,
    agent_release_sha256_x86_64: Option<String>,
    agent_release_sha256_aarch64: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAgentConfigFile {
    agent: RawAgentSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAgentSection {
    node_id: String,
    node_label: String,
    server: String,
    token: String,
    #[serde(default = "default_report_interval_secs")]
    report_interval_secs: u64,
    hostname_override: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

impl RawServerConfigFile {
    fn validate(self) -> Result<ServerConfig, ConfigError> {
        let listen = self
            .server
            .listen
            .parse::<SocketAddr>()
            .map_err(|error| ConfigError::new(format!("invalid server.listen: {error}")))?;
        validate_url(
            "server.public_base_url",
            &self.server.public_base_url,
            &["http", "https"],
        )?;
        if let Some(agent_release_base_url) = self.install.agent_release_base_url.as_deref() {
            validate_url(
                "install.agent_release_base_url",
                agent_release_base_url,
                &["http", "https"],
            )?;
        }
        let agent_release_sha256_x86_64 = self
            .install
            .agent_release_sha256_x86_64
            .map(|value| value.trim().to_string());
        let agent_release_sha256_aarch64 = self
            .install
            .agent_release_sha256_aarch64
            .map(|value| value.trim().to_string());
        if let Some(sha256) = agent_release_sha256_x86_64.as_deref() {
            validate_sha256("install.agent_release_sha256_x86_64", sha256)?;
        }
        if let Some(sha256) = agent_release_sha256_aarch64.as_deref() {
            validate_sha256("install.agent_release_sha256_aarch64", sha256)?;
        }
        if self.install.agent_release_base_url.is_some()
            && (agent_release_sha256_x86_64.is_none() || agent_release_sha256_aarch64.is_none())
        {
            return Err(ConfigError::new(
                "install.agent_release_sha256_x86_64 and install.agent_release_sha256_aarch64 are required when install.agent_release_base_url is configured",
            ));
        }
        let readonly_auth = match (
            self.auth.username.map(|value| value.trim().to_string()),
            self.auth.password.map(|value| value.trim().to_string()),
        ) {
            (Some(username), Some(password)) => {
                validate_non_empty("auth.username", &username)?;
                validate_non_empty("auth.password", &password)?;
                Some(ReadonlyAuthConfig { username, password })
            }
            (None, None) => None,
            (Some(_), None) => {
                return Err(ConfigError::new(
                    "auth.password must be set when auth.username is configured",
                ));
            }
            (None, Some(_)) => {
                return Err(ConfigError::new(
                    "auth.username must be set when auth.password is configured",
                ));
            }
        };

        if self.server.stale_after_secs < 5 {
            return Err(ConfigError::new(
                "server.stale_after_secs must be at least 5 seconds",
            ));
        }
        if self.server.ping_interval_secs < 1 {
            return Err(ConfigError::new(
                "server.ping_interval_secs must be at least 1 second",
            ));
        }
        if self.server.max_message_bytes < 1024 {
            return Err(ConfigError::new(
                "server.max_message_bytes must be at least 1024 bytes",
            ));
        }
        if self.ui.refresh_interval_secs < 1 {
            return Err(ConfigError::new(
                "ui.refresh_interval_secs must be at least 1 second",
            ));
        }
        if readonly_auth.is_none() && !listen.ip().is_loopback() {
            return Err(ConfigError::new(
                "auth.username and auth.password are required when server.listen is not loopback",
            ));
        }

        Ok(ServerConfig {
            listen,
            public_base_url: self.server.public_base_url,
            readonly_auth,
            node_registry_path: self.server.node_registry_path,
            history_db_path: self.server.history_db_path,
            snapshot_path: self.server.snapshot_path,
            stale_after_secs: self.server.stale_after_secs,
            ping_interval_secs: self.server.ping_interval_secs,
            max_message_bytes: self.server.max_message_bytes,
            refresh_interval_secs: self.ui.refresh_interval_secs,
            ignored_filesystems: normalize_string_list(self.filters.ignored_filesystems),
            agent_release_base_url: self.install.agent_release_base_url,
            agent_release_sha256_x86_64,
            agent_release_sha256_aarch64,
        })
    }
}

impl RawAgentConfigFile {
    fn validate(self) -> Result<AgentConfig, ConfigError> {
        validate_identifier("agent.node_id", &self.agent.node_id)?;
        validate_non_empty("agent.node_label", &self.agent.node_label)?;
        validate_url("agent.server", &self.agent.server, &["ws", "wss"])?;
        validate_non_empty("agent.token", &self.agent.token)?;

        if self.agent.report_interval_secs < 1 {
            return Err(ConfigError::new(
                "agent.report_interval_secs must be at least 1 second",
            ));
        }

        if let Some(hostname) = &self.agent.hostname_override {
            validate_non_empty("agent.hostname_override", hostname)?;
        }

        Ok(AgentConfig {
            node_id: self.agent.node_id.trim().to_string(),
            node_label: self.agent.node_label.trim().to_string(),
            server: self.agent.server,
            token: self.agent.token,
            report_interval_secs: self.agent.report_interval_secs,
            hostname_override: self
                .agent
                .hostname_override
                .map(|value| value.trim().to_string()),
            tags: normalize_string_list(self.agent.tags),
        })
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::new(format!("{field} must not be empty")));
    }
    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> Result<(), ConfigError> {
    validate_non_empty(field, value)?;
    if value.len() > 128 {
        return Err(ConfigError::new(format!(
            "{field} must be <= 128 characters"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ConfigError::new(format!(
            "{field} must use only ASCII letters, numbers, '-', '_' or '.'"
        )));
    }
    Ok(())
}

fn validate_url(field: &str, value: &str, schemes: &[&str]) -> Result<(), ConfigError> {
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

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

fn validate_sha256(field: &str, value: &str) -> Result<(), ConfigError> {
    validate_non_empty(field, value)?;
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(ConfigError::new(format!(
            "{field} must be a 64-character hexadecimal SHA-256 digest"
        )));
    }
    Ok(())
}

fn default_history_db_path() -> PathBuf {
    PathBuf::from("./data/history.sqlite3")
}

fn default_node_registry_path() -> PathBuf {
    PathBuf::from("./config/server.json")
}

fn default_snapshot_path() -> PathBuf {
    PathBuf::from("./data/snapshot.json")
}

fn default_stale_after_secs() -> u64 {
    DEFAULT_STALE_AFTER_SECS
}

fn default_ping_interval_secs() -> u64 {
    DEFAULT_PING_INTERVAL_SECS
}

fn default_max_message_bytes() -> usize {
    DEFAULT_MAX_MESSAGE_BYTES
}

fn default_refresh_interval_secs() -> u64 {
    DEFAULT_REFRESH_INTERVAL_SECS
}

fn default_report_interval_secs() -> u64 {
    DEFAULT_REPORT_INTERVAL_SECS
}

fn default_ignored_filesystems() -> Vec<String> {
    vec![
        "devtmpfs".to_string(),
        "overlay".to_string(),
        "tmpfs".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DEFAULT_MAX_MESSAGE_BYTES, parse_agent_config, parse_server_config};

    #[test]
    fn parses_server_config_with_defaults() {
        let config = parse_server_config(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "http://127.0.0.1:8080"
            "#,
        )
        .expect("server config should parse");

        assert_eq!(config.listen.to_string(), "127.0.0.1:8080");
        assert_eq!(config.readonly_auth, None);
        assert_eq!(config.max_message_bytes, DEFAULT_MAX_MESSAGE_BYTES);
        assert_eq!(
            config.node_registry_path,
            PathBuf::from("./config/server.json")
        );
        assert_eq!(
            config.ignored_filesystems,
            vec!["devtmpfs", "overlay", "tmpfs"]
        );
    }

    #[test]
    fn rejects_invalid_server_listen_address() {
        let error = parse_server_config(
            r#"
            [server]
            listen = "oops"
            public_base_url = "http://127.0.0.1:8080"
            "#,
        )
        .expect_err("invalid config should fail");

        assert!(error.to_string().contains("server.listen"));
    }

    #[test]
    fn rejects_invalid_agent_server_scheme() {
        let error = parse_agent_config(
            r#"
            [agent]
            node_id = "hk-01"
            node_label = "Hong Kong 01"
            server = "http://127.0.0.1:8080/ws"
            token = "token"
            "#,
        )
        .expect_err("invalid agent config should fail");

        assert!(error.to_string().contains("agent.server"));
    }

    #[test]
    fn parses_agent_config() {
        let config = parse_agent_config(
            r#"
            [agent]
            node_id = "hk-01"
            node_label = "Hong Kong 01"
            server = "ws://127.0.0.1:8080/ws"
            token = "token"
            report_interval_secs = 7
            hostname_override = "hk-01.internal"
            tags = [" edge ", "apac"]
            "#,
        )
        .expect("agent config should parse");

        assert_eq!(config.node_id, "hk-01");
        assert_eq!(config.report_interval_secs, 7);
        assert_eq!(config.tags, vec!["apac", "edge"]);
    }

    #[test]
    fn parses_server_config_with_install() {
        let config = parse_server_config(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "https://monitor.example.com"
            node_registry_path = "/etc/ximonitor/server.json"

            [auth]
            username = "viewer"
            password = "secret"

            [install]
            agent_release_base_url = "https://downloads.example.com/ximonitor/releases/latest/download"
            agent_release_sha256_x86_64 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            agent_release_sha256_aarch64 = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            "#,
        )
        .expect("server config should parse");

        assert_eq!(
            config
                .readonly_auth
                .as_ref()
                .map(|auth| auth.username.as_str()),
            Some("viewer")
        );
        assert_eq!(
            config.node_registry_path,
            PathBuf::from("/etc/ximonitor/server.json")
        );
        assert_eq!(
            config.agent_release_base_url.as_deref(),
            Some("https://downloads.example.com/ximonitor/releases/latest/download")
        );
        assert_eq!(
            config.agent_release_sha256_x86_64.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            config.agent_release_sha256_aarch64.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
    }

    #[test]
    fn rejects_public_listener_without_auth() {
        let error = parse_server_config(
            r#"
            [server]
            listen = "0.0.0.0:8080"
            public_base_url = "https://monitor.example.com"
            "#,
        )
        .expect_err("public listener without auth should fail");

        assert!(error.to_string().contains("auth.username"));
    }

    #[test]
    fn rejects_install_release_base_without_checksums() {
        let error = parse_server_config(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "https://monitor.example.com"

            [install]
            agent_release_base_url = "https://downloads.example.com/ximonitor/releases/latest/download"
            "#,
        )
        .expect_err("release base without checksums should fail");

        assert!(error.to_string().contains("agent_release_sha256_x86_64"));
    }
}
