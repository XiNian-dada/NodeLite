//! Agent 配置文件加载与原子更新模块。
//!
//! 负责配置文件的读取解析、Token 轮换后的原子写入（写临时文件 -> fsync -> rename -> 目录 fsync -> 权限加固）。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use getrandom::fill as fill_random;
use nodelite_proto::{AgentConfig, parse_agent_config};
use thiserror::Error;
use tokio::fs;
use toml_edit::{DocumentMut, Item, Value};

/// 从磁盘读取并解析 Agent 配置文件。
pub async fn load_agent_config(path: &Path) -> Result<AgentConfig> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    parse_agent_config(&content)
        .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))
}

/// Persistence failures deliberately omit TOML input, which may contain credentials.
#[derive(Debug, Error)]
pub enum TokenPersistenceError {
    #[error("configuration I/O failed")]
    Io(#[from] std::io::Error),
    #[error("configuration is not valid TOML with an agent.token field")]
    InvalidConfig,
    #[error("could not obtain randomness for a private temporary file")]
    Randomness,
    #[error("token persistence task failed")]
    Task(#[from] tokio::task::JoinError),
}

/// Atomically replaces the token and waits for file and parent-directory durability.
pub async fn update_token_in_config(
    config_path: &Path,
    new_token: &str,
) -> std::result::Result<(), TokenPersistenceError> {
    let config_path = config_path.to_path_buf();
    let new_token = new_token.to_string();
    tokio::task::spawn_blocking(move || persist_token_sync(&config_path, &new_token)).await?
}

fn persist_token_sync(
    config_path: &Path,
    new_token: &str,
) -> std::result::Result<(), TokenPersistenceError> {
    let content = std::fs::read_to_string(config_path)?;
    let updated = replace_token_line(&content, new_token)
        .map_err(|_| TokenPersistenceError::InvalidConfig)?;
    let tmp = write_config_payload(&temporary_config_path(config_path)?, &updated)?;
    std::fs::rename(&tmp.0, config_path)?;
    sync_parent_dir(config_path)?;
    Ok(())
}

struct TemporaryConfig(PathBuf);

impl Drop for TemporaryConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn replace_token_line(content: &str, new_token: &str) -> Result<String> {
    let mut document = content
        .parse::<DocumentMut>()
        .map_err(|error| anyhow::anyhow!("failed to parse agent config as TOML: {error}"))?;
    let agent = document
        .get_mut("agent")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("agent section not found in config file"))?;

    if let Some(item) = agent.get_mut("token") {
        let Some(existing_value) = item.as_value_mut() else {
            anyhow::bail!("agent.token is not a value");
        };
        let decor = existing_value.decor().clone();
        *existing_value = Value::from(new_token);
        *existing_value.decor_mut() = decor;
    } else {
        anyhow::bail!("token field not found in config file");
    }

    Ok(document.to_string())
}

fn temporary_config_path(path: &Path) -> std::result::Result<PathBuf, TokenPersistenceError> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("agent.toml");
    let mut suffix = [0u8; 8];
    fill_random(&mut suffix).map_err(|_| TokenPersistenceError::Randomness)?;
    let suffix_hex: String = suffix.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(path.with_file_name(format!("{file_name}.tmp.{suffix_hex}")))
}

fn write_config_payload(path: &Path, payload: &str) -> std::io::Result<TemporaryConfig> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let tmp = TemporaryConfig(path.to_path_buf());
    file.write_all(payload.as_bytes())?;
    file.sync_all()?;
    Ok(tmp)
}

fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use toml_edit::DocumentMut;

    use nodelite_proto::parse_agent_config;

    use super::replace_token_line;

    #[test]
    fn replace_token_line_preserves_comments_and_indent() {
        let input = "[agent]\nnode_id = \"hk-01\"\nnode_label = \"Hong Kong 01\"\nserver = \"ws://127.0.0.1:8080/ws\"\n# token = \"old\"\n token = \"old\" # keep this\n";
        let result = replace_token_line(input, "newvalue").expect("should replace");
        assert!(result.contains("# token = \"old\""));
        assert!(result.contains(" token = \"newvalue\" # keep this"));
        assert_eq!(
            result.matches("token = \"old\"").count(),
            1,
            "only the comment line keeps the old value"
        );
        let parsed = parse_agent_config(&result).expect("updated config should stay valid");
        assert_eq!(parsed.token, "newvalue");
    }

    #[test]
    fn replace_token_line_preserves_multiline_neighbors() {
        let input = r#"[agent]
node_id = "hk-01"
node_label = "Hong Kong 01"
server = "ws://127.0.0.1:8080/ws"
token = "old"

[notes]
description = """
line1
line2
"""
"#;
        let result = replace_token_line(input, "newvalue").expect("should replace token");
        assert!(result.contains("description = \"\"\"\nline1\nline2\n\"\"\""));
        assert!(result.contains("token = \"newvalue\""));
        result
            .parse::<DocumentMut>()
            .expect("updated config should stay valid TOML");
    }

    #[test]
    fn replace_token_line_escapes_special_chars() {
        let result = replace_token_line(
            "[agent]\nnode_id = \"hk-01\"\nnode_label = \"Hong Kong 01\"\nserver = \"ws://127.0.0.1:8080/ws\"\ntoken = \"x\"\n",
            "with\"quote\\and-backslash",
        )
        .expect("ok");
        let parsed = parse_agent_config(&result).expect("updated config should stay valid");
        assert_eq!(parsed.token, "with\"quote\\and-backslash");
    }

    #[test]
    fn replace_token_line_fails_when_no_token_field() {
        let result = replace_token_line("[agent]\nnode_id = \"x\"\n", "new");
        assert!(result.is_err());
    }

    #[test]
    fn temporary_file_collision_does_not_modify_or_remove_existing_file() {
        let path = super::temporary_config_path(&std::env::temp_dir().join("nodelite-collision"))
            .expect("random path");
        std::fs::write(&path, "existing fixture").expect("write collision fixture");
        assert!(super::write_config_payload(&path, "replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("collision preserved"),
            "existing fixture"
        );
        std::fs::remove_file(path).expect("remove collision fixture");
    }
}
