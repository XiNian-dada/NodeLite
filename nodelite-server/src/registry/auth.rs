use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use nodelite_proto::{NodeIdentity, validate_non_empty};
#[cfg(test)]
use tokio::sync::Semaphore;
use tracing::warn;

use super::token::{authorized_node_from_entry, constant_time_eq, verify_token};
use super::validate::validate_runtime_identity;
use super::{
    AuthorizedNode, NodeRegistry, RegisteredNode, RegistryError, RegistryResult,
    RegistryTokenStatus, TOKEN_VERIFY_MAX_PARALLELISM, TOKEN_VERIFY_WAIT_WARN_AFTER,
};

impl NodeRegistry {
    /// 校验 Agent 提交的 Hello 信息与 token,通过后返回"覆盖了注册表里权威字段"的身份
    /// 以及当时的 token 代次, 供 WS 会话后续 hot-path 比较使用。
    pub async fn authorize(
        &self,
        identity: &NodeIdentity,
        token: &str,
    ) -> RegistryResult<AuthorizedNode> {
        validate_runtime_identity(identity)?;
        validate_non_empty("hello.token", token).map_err(RegistryError::validation)?;

        for _ in 0..2 {
            let Some(entry) = self.registered_node(identity.node_id.as_str()).await else {
                return Err(RegistryError::Unauthorized);
            };

            let token_matched = self.token_matches_entry(token, &entry).await?;
            let Some((current_entry, registry_revision)) = self
                .registered_node_snapshot(identity.node_id.as_str())
                .await
            else {
                return Err(RegistryError::Unauthorized);
            };

            if !token_material_matches(&entry, &current_entry) {
                continue;
            }
            if !token_matched {
                return Err(RegistryError::Unauthorized);
            }
            return authorized_node_from_entry(identity, &current_entry, registry_revision);
        }

        warn!(
            node_id = %identity.node_id,
            "registry entry changed repeatedly during token verify; rejecting authorization"
        );
        Err(RegistryError::Unauthorized)
    }

    #[cfg(test)]
    pub async fn is_token_current(&self, node_id: &str, session_generation: u64) -> bool {
        self.token_status(node_id)
            .await
            .is_some_and(|status| status.generation == session_generation)
    }

    /// 返回节点当前 token 状态快照。WS 会话只在 registry revision 变化时调用它,
    /// 平常每帧只比较本地缓存与 atomic revision。
    pub async fn token_status(&self, node_id: &str) -> Option<RegistryTokenStatus> {
        let state = self.state.read().await;
        state
            .entries
            .get(node_id)
            .and_then(|node| token_status_for_node(node, self.registry_revision(), Utc::now()))
    }

    pub(super) async fn registered_node(&self, node_id: &str) -> Option<RegisteredNode> {
        let state = self.state.read().await;
        state.entries.get(node_id).cloned()
    }

    pub(super) async fn registered_node_snapshot(
        &self,
        node_id: &str,
    ) -> Option<(RegisteredNode, u64)> {
        let state = self.state.read().await;
        state
            .entries
            .get(node_id)
            .cloned()
            .map(|entry| (entry, self.registry_revision()))
    }

    async fn token_matches_entry(
        &self,
        input: &str,
        entry: &RegisteredNode,
    ) -> RegistryResult<bool> {
        if !entry.token_hash.is_empty() {
            self.verify_hashed_token(input, &entry.token_hash).await
        } else if !entry.token.is_empty() {
            Ok(constant_time_eq(input, &entry.token))
        } else {
            Ok(false)
        }
    }

    async fn verify_hashed_token(&self, input: &str, token_hash: &str) -> RegistryResult<bool> {
        let wait_started = Instant::now();
        let permit = Arc::clone(&self.token_verify_limiter)
            .acquire_owned()
            .await
            .map_err(|error| {
                RegistryError::internal("token verify limiter closed", anyhow!(error))
            })?;
        let waited = wait_started.elapsed();
        if waited >= TOKEN_VERIFY_WAIT_WARN_AFTER {
            warn!(
                wait_ms = waited.as_millis(),
                limit = self.token_verify_limit,
                "argon2 token verify waited for global concurrency limiter"
            );
        }

        let input = input.to_string();
        let token_hash = token_hash.to_string();
        #[cfg(test)]
        let probe = self.token_verify_probe.clone();

        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            #[cfg(test)]
            let _probe_guard = probe.as_ref().map(|probe| probe.enter());
            verify_token(&input, &token_hash)
        })
        .await
        .map_err(|error| RegistryError::internal("token verify task failed", anyhow!(error)))
    }

    #[cfg(test)]
    pub(super) fn with_token_verify_limit_for_tests(mut self, max_parallel: usize) -> Self {
        assert!(max_parallel > 0, "test token verify limit must be positive");
        self.token_verify_limit = max_parallel;
        self.token_verify_limiter = Arc::new(Semaphore::new(max_parallel));
        self
    }

    #[cfg(test)]
    pub(super) fn with_token_verify_probe_for_tests(
        mut self,
        probe: Arc<TokenVerifyProbe>,
    ) -> Self {
        self.token_verify_probe = Some(probe);
        self
    }
}

pub(super) fn token_status_for_node(
    node: &RegisteredNode,
    registry_revision: u64,
    now: DateTime<Utc>,
) -> Option<RegistryTokenStatus> {
    if !super::token::token_is_unexpired(node, now) {
        return None;
    }

    Some(RegistryTokenStatus {
        generation: node.token_generation,
        token_expires_at: node.token_expires_at,
        registry_revision,
    })
}

pub(super) fn default_token_verify_limit() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(TOKEN_VERIFY_MAX_PARALLELISM))
        .unwrap_or(1)
        .max(1)
}

fn token_material_matches(left: &RegisteredNode, right: &RegisteredNode) -> bool {
    left.token_generation == right.token_generation
        && left.token_hash == right.token_hash
        && constant_time_eq(&left.token, &right.token)
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct TokenVerifyProbe {
    active: AtomicUsize,
    max_active: AtomicUsize,
    delay: Duration,
}

#[cfg(test)]
impl TokenVerifyProbe {
    pub(super) fn new(delay: Duration) -> Self {
        Self {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            delay,
        }
    }

    pub(super) fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }

    fn enter(&self) -> TokenVerifyProbeGuard<'_> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_max_active(active);
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        TokenVerifyProbeGuard { probe: self }
    }

    fn record_max_active(&self, active: usize) {
        let mut observed = self.max_active.load(Ordering::SeqCst);
        while active > observed {
            match self.max_active.compare_exchange(
                observed,
                active,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(next_observed) => observed = next_observed,
            }
        }
    }
}

#[cfg(test)]
struct TokenVerifyProbeGuard<'a> {
    probe: &'a TokenVerifyProbe,
}

#[cfg(test)]
impl Drop for TokenVerifyProbeGuard<'_> {
    fn drop(&mut self) {
        self.probe.active.fetch_sub(1, Ordering::SeqCst);
    }
}
