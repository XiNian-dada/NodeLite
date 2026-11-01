//! 节点服务与流量套餐元数据的部分更新。
//!
//! 请求里缺省的字段保持原值，显式 `null` 才清空。否则只改到期日的请求会把计费口径重置为
//! 默认值，并清掉流量上限与限速。

use std::sync::Arc;

use chrono::{DateTime, Utc};
use nodelite_proto::validate_identifier;

use crate::sanitize::sanitize_renewal_price;

use super::storage::mutate_registry_file;
use super::{
    MAX_TRAFFIC_THROTTLE_KBPS, NodeRegistry, NodeServiceMetadata, RegisteredNode, RegistryError,
    RegistryResult, TrafficAccounting,
};

/// `None` keeps the stored value; `Some(None)` clears a nullable field.
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeServiceMetadataPatch {
    pub(crate) service_expires_at: Option<Option<DateTime<Utc>>>,
    pub(crate) service_unlimited: Option<bool>,
    pub(crate) renewal_price: Option<Option<String>>,
    pub(crate) traffic_limit_bytes: Option<Option<u64>>,
    pub(crate) traffic_accounting: Option<TrafficAccounting>,
    pub(crate) traffic_throttle_kbps: Option<Option<u64>>,
}

impl From<NodeServiceMetadata> for NodeServiceMetadataPatch {
    fn from(metadata: NodeServiceMetadata) -> Self {
        Self {
            service_expires_at: Some(metadata.service_expires_at),
            service_unlimited: Some(metadata.service_unlimited),
            renewal_price: Some(metadata.renewal_price),
            traffic_limit_bytes: Some(metadata.traffic_limit_bytes),
            traffic_accounting: Some(metadata.traffic_accounting),
            traffic_throttle_kbps: Some(metadata.traffic_throttle_kbps),
        }
    }
}

impl NodeRegistry {
    #[cfg(test)]
    pub(crate) async fn update_service_metadata(
        &self,
        node_id: &str,
        metadata: NodeServiceMetadata,
    ) -> RegistryResult<RegisteredNode> {
        self.update_service_metadata_patch(node_id, metadata.into())
            .await
    }

    pub(crate) async fn update_service_metadata_patch(
        &self,
        node_id: &str,
        mut patch: NodeServiceMetadataPatch,
    ) -> RegistryResult<RegisteredNode> {
        validate_identifier("node_id", node_id).map_err(RegistryError::validation)?;
        if let Some(renewal_price) = patch.renewal_price.take() {
            patch.renewal_price =
                Some(sanitize_renewal_price(renewal_price).map_err(RegistryError::validation)?);
        }
        // Only a submitted zero is rejected: registries already holding one must still load
        // and accept edits to other fields.
        if patch.traffic_limit_bytes == Some(Some(0)) {
            return Err(RegistryError::validation(
                "traffic_limit_bytes must be at least 1",
            ));
        }
        let node_id = node_id.to_string();
        let path = Arc::clone(&self.path);
        let min_version = self.committed_version().await;
        let (node, file) = mutate_registry_file(path.as_ref(), min_version, move |file| {
            let Some(node) = file.nodes.iter_mut().find(|node| node.node_id == node_id) else {
                return Err(RegistryError::NodeNotFound(node_id.clone()));
            };
            let patch = patch.clone();
            if let Some(value) = patch.service_unlimited {
                node.service_unlimited = value;
            }
            if let Some(value) = patch.service_expires_at {
                node.service_expires_at = value;
            }
            if node.service_unlimited {
                node.service_expires_at = None;
            }
            if let Some(value) = patch.renewal_price {
                node.renewal_price = value;
            }
            if let Some(value) = patch.traffic_limit_bytes {
                node.traffic_limit_bytes = value;
            }
            if let Some(value) = patch.traffic_accounting {
                node.traffic_accounting = value;
            }
            if let Some(value) = patch.traffic_throttle_kbps {
                node.traffic_throttle_kbps = value;
            }
            validate_traffic_combination(node.traffic_limit_bytes, node.traffic_throttle_kbps)?;
            super::validate::validate_registered_node(node)?;
            Ok((node.clone(), true))
        })
        .await?;

        self.replace_state_from_file(file).await?;
        Ok(node)
    }
}

fn validate_traffic_combination(
    traffic_limit_bytes: Option<u64>,
    traffic_throttle_kbps: Option<u64>,
) -> RegistryResult<()> {
    if traffic_throttle_kbps.is_some() && traffic_limit_bytes.is_none() {
        return Err(RegistryError::validation(
            "traffic_throttle_kbps requires traffic_limit_bytes",
        ));
    }
    if let Some(rate) = traffic_throttle_kbps
        && (rate == 0 || rate > MAX_TRAFFIC_THROTTLE_KBPS)
    {
        return Err(RegistryError::validation(format!(
            "traffic_throttle_kbps must be between 1 and {MAX_TRAFFIC_THROTTLE_KBPS}"
        )));
    }
    Ok(())
}
