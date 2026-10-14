//! Agent 配置文件加载与原子更新模块。
//!
//! 凭证必须同步文件和目录后才能用于握手；配置锁跨阻塞任务取消，防止重连读到未完成的写入。

use std::path::{Path, PathBuf};

use anyhow::Result;
use getrandom::fill as fill_random;
use nodelite_proto::{AgentConfig, parse_agent_config};
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Value};

static CONFIG_IO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 从磁盘读取并解析 Agent 配置文件。
pub async fn load_agent_config(
    path: &Path,
) -> std::result::Result<AgentConfig, TokenPersistenceError> {
    let _guard = CONFIG_IO_LOCK.lock().await;
    let saved_path = path.to_path_buf();
    let content = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(&saved_path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        file.sync_all()?;
        sync_parent_dir(&saved_path)?;
        Ok(content)
    })
    .await??;
    parse_agent_config(&content).map_err(|_| TokenPersistenceError::InvalidConfig)
}

/// Persistence failures deliberately omit TOML input, which may contain credentials.
#[derive(Debug, Error)]
pub enum TokenPersistenceError {
    #[error("configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration is invalid or missing required Agent fields")]
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
    update_token_with_sync(config_path, new_token, sync_parent_dir).await
}

async fn update_token_with_sync(
    config_path: &Path,
    new_token: &str,
    sync: impl FnOnce(&Path) -> std::io::Result<()> + Send + 'static,
) -> std::result::Result<(), TokenPersistenceError> {
    let config_path = config_path.to_path_buf();
    let new_token = new_token.to_string();
    let guard = CONFIG_IO_LOCK.lock().await;
    tokio::task::spawn_blocking(move || {
        // A cancelled session must not reconnect before its blocking write finishes.
        let _guard = guard;
        persist_token_with_sync(&config_path, &new_token, sync)
    })
    .await?
}

fn persist_token_with_sync(
    config_path: &Path,
    new_token: &str,
    sync: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::result::Result<(), TokenPersistenceError> {
    let content = std::fs::read_to_string(config_path)?;
    let updated = replace_token_line(&content, new_token)
        .map_err(|_| TokenPersistenceError::InvalidConfig)?;
    let tmp = write_config_payload(&temporary_config_path(config_path)?, &updated)?;
    std::fs::rename(&tmp.0, config_path)?;
    sync(config_path)?;
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
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

    #[tokio::test]
    async fn published_token_is_not_confirmed_when_parent_sync_fails() {
        let path =
            super::temporary_config_path(&std::env::temp_dir().join("nodelite-sync-failure"))
                .expect("random fixture path");
        std::fs::write(&path, "[agent]\nnode_id = \"sync-01\"\nnode_label = \"Sync\"\nserver = \"ws://127.0.0.1/ws\"\ntoken = \"old-fixture\"\n").expect("fixture");
        let result = super::persist_token_with_sync(&path, "new-fixture", |_| {
            Err(std::io::Error::other("injected directory fsync failure"))
        });
        assert!(
            result.is_err(),
            "publication alone must not produce a success acknowledgement"
        );
        assert!(
            std::fs::read_to_string(&path)
                .expect("published file")
                .contains("new-fixture")
        );
        let saved = super::load_agent_config(&path)
            .await
            .expect("recovery must successfully resync file and directory");
        assert_eq!(saved.token, "new-fixture");
        std::fs::remove_file(path).expect("cleanup fixture");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_write_keeps_its_lock_until_durability_and_reconnect_read_completes() {
        use std::sync::{Arc, Condvar, Mutex};
        let path =
            super::temporary_config_path(&std::env::temp_dir().join("nodelite-cancel-write"))
                .expect("random fixture path");
        let _fixture = super::write_config_payload(&path,
            "[agent]\nnode_id = \"cancel-01\"\nnode_label = \"Cancel\"\nserver = \"ws://127.0.0.1/ws\"\ntoken = \"old-fixture\"\n").expect("private fixture");
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_path = path.clone();
        let worker_started = Arc::clone(&started);
        let worker_release = Arc::clone(&release);
        let write = tokio::spawn(async move {
            super::update_token_with_sync(&worker_path, "new-fixture", move |path| {
                worker_started.notify_one();
                let (lock, ready) = &*worker_release;
                let mut released = lock.lock().expect("test lock");
                while !*released {
                    released = ready.wait(released).expect("test barrier");
                }
                super::sync_parent_dir(path)
            })
            .await
        });
        started.notified().await;
        write.abort();
        let _ = write.await;
        let mut read = tokio::spawn(async move { super::load_agent_config(&path).await });
        let blocked = tokio::time::timeout(std::time::Duration::from_millis(20), &mut read)
            .await
            .is_err();
        *release.0.lock().expect("release lock") = true;
        release.1.notify_one();
        assert!(blocked, "reconnect read must wait for the abandoned write");
        assert_eq!(
            read.await
                .expect("reader task")
                .expect("durable config")
                .token,
            "new-fixture"
        );
    }
}
