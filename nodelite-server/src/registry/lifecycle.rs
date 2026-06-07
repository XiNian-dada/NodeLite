use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use nodelite_proto::validate_non_empty;
use tokio::sync::{RwLock, Semaphore};

use super::auth::default_token_verify_limit;
use super::storage::{
    load_registry_state, load_registry_state_with_fingerprint, mutate_registry_file,
    registry_file_fingerprint,
};
use super::token::{constant_time_eq, generate_token, hash_token, prune_expired_install_sessions};
use super::{
    ConsumedInstall, DEFAULT_TOKEN_VALIDITY_DAYS, NodeRegistry, RegisteredNode, RegistryError,
    RegistryFile, RegistryReloadCheckpoint, RegistryResult,
};

impl NodeRegistry {
    /// 从磁盘加载注册表;文件不存在时返回空注册表(首次部署的合理状态)。
    pub async fn load(path: &Path) -> RegistryResult<Self> {
        let (state, fingerprint) = load_registry_state_with_fingerprint(path).await?;
        let token_verify_limit = default_token_verify_limit();

        Ok(Self {
            path: Arc::new(path.to_path_buf()),
            state: Arc::new(RwLock::new(state)),
            reload_checkpoint: Arc::new(RwLock::new(RegistryReloadCheckpoint {
                fingerprint: Some(fingerprint),
            })),
            registry_revision: Arc::new(AtomicU64::new(1)),
            token_verify_limit,
            token_verify_limiter: Arc::new(Semaphore::new(token_verify_limit)),
            #[cfg(test)]
            token_verify_probe: None,
        })
    }

    /// 当前注册表状态版本。任一注册表 reload / 写入导致的内存状态变化都会递增。
    pub fn registry_revision(&self) -> u64 {
        self.registry_revision.load(Ordering::Acquire)
    }

    /// 刷新节点的 Token:生成新明文 token, 哈希入库,代次 +1, 延长过期时间。
    /// 返回 (new_plaintext_token, expires_at, new_generation)。明文只在
    /// 进程内存里短暂存在,从这里被传递给 WS 端发送给 agent。
    pub async fn refresh_token(
        &self,
        node_id: &str,
    ) -> RegistryResult<(String, DateTime<Utc>, u64)> {
        let path = Arc::clone(&self.path);
        let node_id = node_id.to_string();
        let ((new_token, expires_at, generation), _) =
            mutate_registry_file(path.as_ref(), move |file| {
                let now = Utc::now();
                let Some(node) = file.nodes.iter_mut().find(|n| n.node_id == node_id) else {
                    return Err(RegistryError::NodeNotFound(node_id.clone()));
                };

                let new_token = generate_token()?;
                let expires_at = now + ChronoDuration::days(DEFAULT_TOKEN_VALIDITY_DAYS);
                node.token_hash = hash_token(&new_token).map_err(|error| {
                    RegistryError::internal("failed to hash refreshed token", error.into())
                })?;
                node.token_generation = node.token_generation.saturating_add(1);
                node.token_expires_at = Some(expires_at);
                // 升级路径残留的明文也在这里清空,确保从此刻起 disk 上彻底无明文。
                node.token.clear();

                Ok(((new_token, expires_at, node.token_generation), true))
            })
            .await?;

        self.reload().await?;

        Ok((new_token, expires_at, generation))
    }

    /// 从磁盘重新加载注册表。返回 `Ok(true)` 表示发现了变化。
    pub async fn reload(&self) -> RegistryResult<bool> {
        let fingerprint = registry_file_fingerprint(self.path.as_path()).await?;
        self.reload_from_disk(fingerprint).await
    }

    pub(crate) async fn reload_if_file_changed(&self) -> RegistryResult<bool> {
        let fingerprint = registry_file_fingerprint(self.path.as_path()).await?;
        {
            let checkpoint = self.reload_checkpoint.read().await;
            if checkpoint.fingerprint == Some(fingerprint) {
                return Ok(false);
            }
        }

        self.reload_from_disk(fingerprint).await
    }

    async fn reload_from_disk(
        &self,
        fingerprint: super::storage::RegistryFileFingerprint,
    ) -> RegistryResult<bool> {
        let next_state = load_registry_state(self.path.as_path()).await?;
        let mut state = self.state.write().await;
        let changed = *state != next_state;
        if changed {
            self.bump_registry_revision();
            *state = next_state;
        }
        drop(state);

        let mut checkpoint = self.reload_checkpoint.write().await;
        checkpoint.fingerprint = Some(fingerprint);
        Ok(changed)
    }

    /// 已登记的节点数量。
    pub async fn count(&self) -> usize {
        let state = self.state.read().await;
        state.entries.len()
    }

    /// 返回注册表中的节点条目,但不会暴露 token 字符串。
    ///
    /// 设置页需要查看 token 到期时间与登记标签;这些信息来自注册表而不是
    /// 运行态快照。调用方负责只序列化安全字段,不要把 `token` 下发给浏览器。
    pub async fn list_registered_nodes(&self) -> Vec<RegisteredNode> {
        let state = self.state.read().await;
        let mut nodes: Vec<_> = state.entries.values().cloned().collect();
        nodes.sort_by(|left, right| {
            left.node_label
                .cmp(&right.node_label)
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        nodes
    }

    /// 返回当前注册表里的全部 node_id,用于跨模块做被动清理。
    pub async fn node_ids(&self) -> Vec<String> {
        let state = self.state.read().await;
        let mut node_ids: Vec<_> = state.entries.keys().cloned().collect();
        node_ids.sort();
        node_ids
    }

    /// 一次性消费安装令牌:成功时返回对应的 `RegisteredNode` **以及**该节点
    /// 当前明文 session token —— 后者由 install_session 在颁发时短暂持有,
    /// 这是 #56 之后整个系统里唯一返回明文的入口。一旦本函数返回,
    /// install_session(连同明文)即被从注册表删除。
    pub async fn consume_install_token(
        &self,
        token: &str,
    ) -> RegistryResult<Option<ConsumedInstall>> {
        validate_non_empty("install token", token).map_err(RegistryError::validation)?;

        let token = token.to_string();
        let (result, file) = mutate_registry_file(self.path.as_path(), move |file| {
            let pruned = prune_expired_install_sessions(file, Utc::now());
            let Some(index) = file
                .install_sessions
                .iter()
                .position(|session| constant_time_eq(&session.token, &token))
            else {
                return Ok((None, pruned));
            };

            let session = file.install_sessions.remove(index);
            let node = file
                .nodes
                .iter()
                .find(|node| node.node_id == session.node_id)
                .cloned();
            let result = node.map(|node| ConsumedInstall {
                node,
                node_session_token: session.node_session_token,
            });
            Ok((result, true))
        })
        .await?;
        self.replace_state_from_file(file).await?;
        Ok(result)
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub(super) async fn replace_state_from_file(&self, file: RegistryFile) -> RegistryResult<()> {
        let state = super::storage::load_registry_state_from_file(self.path.as_path(), file)?;
        let mut guard = self.state.write().await;
        self.bump_registry_revision();
        *guard = state;
        Ok(())
    }

    pub(super) fn bump_registry_revision(&self) {
        self.registry_revision.fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(test)]
    pub(crate) async fn hold_state_write_lock_for_test(
        &self,
        acquired: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    ) {
        let _guard = self.state.write().await;
        let _ = acquired.send(());
        let _ = release.await;
    }
}
