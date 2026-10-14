//! Two-phase credential renewal stores only candidate hashes until durable Agent confirmation.

use chrono::{DateTime, Duration, Utc};
use nodelite_proto::validate_identifier;

use super::auth::token_status_for_node;
use super::storage::mutate_registry_file;
use super::token::{generate_token, hash_token};
use super::{
    DEFAULT_TOKEN_VALIDITY_DAYS, NodeRegistry, PendingToken, RegistryError, RegistryResult,
    TOKEN_REFRESH_GRACE_MINUTES,
};

pub(crate) const MAX_PENDING_TOKENS: usize = 8;

pub(crate) struct PreparedToken {
    pub token: String,
    pub refresh_id: String,
    pub generation: u64,
    pub expires_at: DateTime<Utc>,
}

impl NodeRegistry {
    #[cfg(test)]
    pub async fn refresh_token(
        &self,
        node_id: &str,
        generation: u64,
    ) -> RegistryResult<(String, DateTime<Utc>, u64)> {
        let candidate = self.prepare_token_refresh(node_id, generation).await?;
        let (generation, expires_at) = self
            .commit_token_refresh(node_id, &candidate.refresh_id)
            .await?;
        Ok((candidate.token, expires_at, generation))
    }
    pub(crate) async fn prepare_token_refresh(
        &self,
        node_id: &str,
        generation: u64,
    ) -> RegistryResult<PreparedToken> {
        validate_identifier("node_id", node_id).map_err(RegistryError::validation)?;
        let node_id = node_id.to_string();
        let (prepared, _) = mutate_registry_file(self.path.as_ref(), move |file| {
            let node = file.nodes.iter_mut().find(|n| n.node_id == node_id)
                .ok_or_else(|| RegistryError::NodeNotFound(node_id.clone()))?;
            let now = Utc::now();
            if token_status_for_node(node, generation, 0, now).is_none() {
                return Err(RegistryError::Unauthorized);
            }
            node.pending_tokens.retain(|pending| now < pending.expires_at);
            if node.pending_tokens.len() >= MAX_PENDING_TOKENS {
                return Err(RegistryError::validation("unconfirmed credential limit reached; retry with a saved candidate or reissue the node"));
            }
            let next = node.pending_tokens.iter().map(|p| p.generation)
                .chain(std::iter::once(node.token_generation)).max().unwrap_or(node.token_generation)
                .checked_add(1).ok_or_else(|| RegistryError::validation("token generation exhausted"))?;
            let token = generate_token()?;
            let refresh_id = generate_token()?;
            let expires_at = now + Duration::days(DEFAULT_TOKEN_VALIDITY_DAYS);
            let token_hash = hash_token(&token)
                .map_err(|e| RegistryError::internal("failed to hash candidate token", e.into()))?;
            node.pending_tokens.push(PendingToken { refresh_id: refresh_id.clone(), token_hash,
                generation: next, expires_at });
            Ok((PreparedToken { token, refresh_id, generation: next, expires_at }, true))
        }).await?;
        self.reload().await?;
        Ok(prepared)
    }

    pub(crate) async fn commit_token_refresh(
        &self,
        node_id: &str,
        refresh_id: &str,
    ) -> RegistryResult<(u64, DateTime<Utc>)> {
        crate::sanitize::validate_refresh_id(refresh_id).map_err(RegistryError::validation)?;
        let node_id = node_id.to_string();
        let refresh_id = refresh_id.to_string();
        let (committed, _) = mutate_registry_file(self.path.as_ref(), move |file| {
            let node = file
                .nodes
                .iter_mut()
                .find(|n| n.node_id == node_id)
                .ok_or(RegistryError::Unauthorized)?;
            let now = Utc::now();
            let candidate = node
                .pending_tokens
                .iter()
                .find(|p| p.refresh_id == refresh_id && now < p.expires_at)
                .cloned()
                .ok_or(RegistryError::Unauthorized)?;
            node.previous_token_hash =
                std::mem::replace(&mut node.token_hash, candidate.token_hash);
            node.previous_token_generation = Some(node.token_generation);
            node.previous_token_valid_until = Some(
                node.token_expires_at
                    .map(|expiry| expiry.min(now + Duration::minutes(TOKEN_REFRESH_GRACE_MINUTES)))
                    .unwrap_or(now + Duration::minutes(TOKEN_REFRESH_GRACE_MINUTES)),
            );
            node.token_generation = candidate.generation;
            node.token_expires_at = Some(candidate.expires_at);
            node.token.clear();
            node.pending_tokens.clear();
            Ok(((candidate.generation, candidate.expires_at), true))
        })
        .await?;
        self.reload().await?;
        self.token_cache.lock().clear();
        Ok(committed)
    }

    pub(crate) async fn activate_saved_token(
        &self,
        node_id: &str,
        generation: u64,
    ) -> RegistryResult<()> {
        let entry = self
            .registered_node(node_id)
            .await
            .ok_or(RegistryError::Unauthorized)?;
        if let Some(pending) = entry
            .pending_tokens
            .iter()
            .find(|p| p.generation == generation)
        {
            self.commit_token_refresh(node_id, &pending.refresh_id)
                .await?;
        }
        Ok(())
    }
}
