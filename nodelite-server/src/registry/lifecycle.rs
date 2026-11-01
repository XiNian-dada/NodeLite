//! 节点注册表生命周期管理与热重载模块。
//!
//! 提供内存注册表状态初始化、基于 mtime/size 指纹的热重载、节点注销、元数据更新与流量配额调整。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use lru::LruCache;
use nodelite_proto::{validate_identifier, validate_non_empty};
use parking_lot::Mutex as ParkingLotMutex;
use tokio::sync::{RwLock, Semaphore};

use crate::sanitize::sanitize_location_override;

use super::storage::{
    load_registry_state, load_registry_state_with_fingerprint, mutate_registry_file,
    registry_file_fingerprint,
};
use super::token::{constant_time_eq, prune_expired_install_sessions};
use super::{
    ConsumedInstall, NodeRegistry, RegisteredNode, RegistryError, RegistryFile,
    RegistryReloadCheckpoint, RegistryResult, RegistryState, TOKEN_CACHE_CAPACITY, TrafficQuota,
    coordinate_to_microdegrees,
};

impl NodeRegistry {
    /// 从磁盘加载注册表;文件不存在时返回空注册表(首次部署的合理状态)。
    #[cfg(test)]
    pub async fn load(path: &Path) -> RegistryResult<Self> {
        Self::load_with_token_verify_limit(
            path,
            nodelite_proto::DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM,
        )
        .await
    }

    /// 从磁盘加载注册表,并使用已校验的 Argon2 token 验证并发上限。
    pub async fn load_with_token_verify_limit(
        path: &Path,
        token_verify_limit: usize,
    ) -> RegistryResult<Self> {
        if !(nodelite_proto::MIN_TOKEN_VERIFY_MAX_PARALLELISM
            ..=nodelite_proto::MAX_TOKEN_VERIFY_MAX_PARALLELISM)
            .contains(&token_verify_limit)
        {
            return Err(RegistryError::invalid_config(
                "token_verify_max_parallelism",
                format!(
                    "must be between {} and {}",
                    nodelite_proto::MIN_TOKEN_VERIFY_MAX_PARALLELISM,
                    nodelite_proto::MAX_TOKEN_VERIFY_MAX_PARALLELISM,
                ),
            ));
        }
        let (state, fingerprint) = load_registry_state_with_fingerprint(path).await?;

        Ok(Self {
            path: Arc::new(path.to_path_buf()),
            state: Arc::new(RwLock::new(state)),
            reload_checkpoint: Arc::new(RwLock::new(RegistryReloadCheckpoint {
                fingerprint: Some(fingerprint),
            })),
            registry_revision: Arc::new(AtomicU64::new(1)),
            token_verify_limit,
            token_verify_limiter: Arc::new(Semaphore::new(token_verify_limit)),
            token_verify_metrics: Arc::new(super::metrics::TokenVerifyMetricsState::default()),
            token_cache: Arc::new(ParkingLotMutex::new(LruCache::new(
                std::num::NonZeroUsize::new(TOKEN_CACHE_CAPACITY).expect("cache capacity > 0"),
            ))),
            #[cfg(test)]
            token_verify_probe: None,
        })
    }

    /// 当前注册表状态版本。任一注册表 reload / 写入导致的内存状态变化都会递增。
    pub fn registry_revision(&self) -> u64 {
        self.registry_revision.load(Ordering::Acquire)
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
        use super::storage::RegistryFileFingerprint;

        let next_state = load_registry_state(self.path.as_path()).await?;
        let (memory_version, memory_has_nodes) = {
            let state = self.state.read().await;
            (state.version, !state.entries.is_empty())
        };
        if fingerprint == RegistryFileFingerprint::Missing
            && (memory_version > 0 || memory_has_nodes)
        {
            return Err(RegistryError::diverged(
                self.path.as_path(),
                None,
                memory_version,
            ));
        }
        if next_state.version < memory_version {
            // 本进程的写入可能在读取后提交并先行应用；文件指纹变化说明这是竞态而非回退，留给下一轮。
            if registry_file_fingerprint(self.path.as_path()).await? != fingerprint {
                return Ok(false);
            }
            return Err(RegistryError::diverged(
                self.path.as_path(),
                Some(next_state.version),
                memory_version,
            ));
        }
        Ok(self.apply_loaded_state(next_state, Some(fingerprint)).await)
    }

    /// 内存中已应用的最高磁盘提交版本，写入以它为下限防止覆盖回退文件。
    pub(super) async fn committed_version(&self) -> u64 {
        self.state.read().await.version
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

    /// 从注册表移除节点，并一并撤销尚未消费的安装令牌。
    pub async fn remove_node(&self, node_id: &str) -> RegistryResult<RegisteredNode> {
        validate_identifier("node_id", node_id).map_err(RegistryError::validation)?;

        let node_id = node_id.to_string();
        let path = Arc::clone(&self.path);
        let min_version = self.committed_version().await;
        let (removed, file) = mutate_registry_file(path.as_ref(), min_version, move |file| {
            prune_expired_install_sessions(file, Utc::now());
            let Some(index) = file.nodes.iter().position(|node| node.node_id == node_id) else {
                return Err(RegistryError::NodeNotFound(node_id.clone()));
            };

            let removed = file.nodes.remove(index);
            file.install_sessions
                .retain(|session| session.node_id != removed.node_id);
            Ok((removed, true))
        })
        .await?;

        self.replace_state_from_file(file).await?;
        self.token_cache.lock().clear();
        Ok(removed)
    }

    /// 返回节点启用中的流量套餐；未设置上限的节点不参与用量累计。
    pub(crate) async fn traffic_quota(&self, node_id: &str) -> Option<TrafficQuota> {
        self.state
            .read()
            .await
            .entries
            .get(node_id)
            .and_then(|node| {
                node.traffic_limit_bytes.map(|limit_bytes| TrafficQuota {
                    limit_bytes,
                    accounting: node.traffic_accounting,
                    throttle_kbps: node.traffic_throttle_kbps,
                })
            })
    }

    /// 更新设置页展示用的节点运营元数据。
    /// 更新设置页展示与地图落点使用的手动位置覆盖。
    pub async fn update_location_override(
        &self,
        node_id: &str,
        country: Option<String>,
        city: Option<String>,
        latitude: Option<f64>,
        longitude: Option<f64>,
    ) -> RegistryResult<RegisteredNode> {
        validate_identifier("node_id", node_id).map_err(RegistryError::validation)?;
        let location = sanitize_location_override(country, city, latitude, longitude)
            .map_err(RegistryError::validation)?;
        let node_id = node_id.to_string();
        let path = Arc::clone(&self.path);
        let min_version = self.committed_version().await;
        let (node, file) = mutate_registry_file(path.as_ref(), min_version, move |file| {
            let Some(node) = file.nodes.iter_mut().find(|node| node.node_id == node_id) else {
                return Err(RegistryError::NodeNotFound(node_id.clone()));
            };
            match location.as_ref() {
                Some(location) => {
                    node.location_override_country = Some(location.country.clone());
                    node.location_override_city = location.city.clone();
                    node.location_override_latitude_microdegrees =
                        location.latitude.map(coordinate_to_microdegrees);
                    node.location_override_longitude_microdegrees =
                        location.longitude.map(coordinate_to_microdegrees);
                }
                None => {
                    node.location_override_country = None;
                    node.location_override_city = None;
                    node.location_override_latitude_microdegrees = None;
                    node.location_override_longitude_microdegrees = None;
                }
            }
            super::validate::validate_registered_node(node)?;
            Ok((node.clone(), true))
        })
        .await?;

        self.replace_state_from_file(file).await?;
        Ok(node)
    }

    pub async fn list_location_overrides(
        &self,
    ) -> Vec<(String, Option<nodelite_proto::GeoIpLocation>)> {
        let state = self.state.read().await;
        state
            .entries
            .values()
            .map(|node| (node.node_id.clone(), node.location_override()))
            .collect()
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
        let min_version = self.committed_version().await;
        let (result, file) = mutate_registry_file(self.path.as_path(), min_version, move |file| {
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
        self.apply_loaded_state(state, None).await;
        Ok(())
    }

    pub(super) async fn apply_loaded_state(
        &self,
        next_state: RegistryState,
        fingerprint: Option<super::storage::RegistryFileFingerprint>,
    ) -> bool {
        let mut state = self.state.write().await;
        // Commit order, not completion order, controls authorization and the reload checkpoint.
        if next_state.version < state.version {
            return false;
        }
        let changed = *state != next_state;
        if changed {
            *state = next_state;
            self.bump_registry_revision();
        }
        self.reload_checkpoint.write().await.fingerprint = fingerprint;
        changed
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
