use std::collections::HashMap;
use std::path::Path;

use nodelite_proto::{
    MAX_NODE_IDENTITY_TEXT_BYTES, MAX_NODE_TAG_BYTES, MAX_NODE_TAGS, NodeIdentity,
    validate_bounded_text, validate_identifier, validate_non_empty, validate_tag_list,
};

use crate::sanitize::{validate_location_override, validate_renewal_price};

use super::{InstallSession, RegisteredNode, RegistryError, RegistryFile, RegistryResult};

pub(super) fn validate_registry_file(path: &Path, file: &RegistryFile) -> RegistryResult<()> {
    let mut seen_nodes = HashMap::with_capacity(file.nodes.len());
    for node in &file.nodes {
        validate_registered_node(node)?;
        if seen_nodes.insert(node.node_id.as_str(), ()).is_some() {
            return Err(RegistryError::validation(format!(
                "duplicate node_id {} in {}",
                node.node_id,
                path.display()
            )));
        }
    }
    let mut seen_install_tokens = HashMap::with_capacity(file.install_sessions.len());
    for session in &file.install_sessions {
        validate_install_session(session)?;
        if !seen_nodes.contains_key(session.node_id.as_str()) {
            return Err(RegistryError::validation(format!(
                "install token for unknown node_id {} in {}",
                session.node_id,
                path.display()
            )));
        }
        if seen_install_tokens
            .insert(session.token.as_str(), ())
            .is_some()
        {
            return Err(RegistryError::validation(format!(
                "duplicate install token in {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_registered_node(node: &RegisteredNode) -> RegistryResult<()> {
    validate_identifier("node.node_id", &node.node_id).map_err(RegistryError::validation)?;
    validate_bounded_text(
        "node.node_label",
        &node.node_label,
        MAX_NODE_IDENTITY_TEXT_BYTES,
    )
    .map_err(RegistryError::validation)?;
    // 注册表中 token 必须以哈希形式存在; 旧版本的明文 `token` 字段
    // 在 `migrate_legacy_tokens` 中已经被搬迁过来。
    if node.token_hash.is_empty() && node.token.is_empty() {
        return Err(RegistryError::validation("node.token_hash is empty"));
    }
    validate_tag_list("node.tags", &node.tags, MAX_NODE_TAGS, MAX_NODE_TAG_BYTES)
        .map_err(RegistryError::validation)?;
    if let Some(price) = node.renewal_price.as_deref() {
        validate_renewal_price(price).map_err(RegistryError::validation)?;
    }
    validate_location_override_fields(node)?;
    Ok(())
}

fn validate_location_override_fields(node: &RegisteredNode) -> RegistryResult<()> {
    let has_location = node.location_override_country.is_some()
        || node.location_override_city.is_some()
        || node.location_override_latitude_microdegrees.is_some()
        || node.location_override_longitude_microdegrees.is_some();
    if !has_location {
        return Ok(());
    }
    let Some(location) = node.location_override() else {
        return Err(RegistryError::validation(
            "node.location_override_country is required when location override is set",
        ));
    };
    if node.location_override_latitude_microdegrees.is_some()
        != node.location_override_longitude_microdegrees.is_some()
    {
        return Err(RegistryError::validation(
            "node location override latitude and longitude must be set together",
        ));
    }
    validate_location_override(&location).map_err(RegistryError::validation)
}

fn validate_install_session(session: &InstallSession) -> RegistryResult<()> {
    validate_non_empty("install_session.token", &session.token)
        .map_err(RegistryError::validation)?;
    validate_identifier("install_session.node_id", &session.node_id)
        .map_err(RegistryError::validation)?;
    Ok(())
}

pub(crate) fn validate_runtime_identity(identity: &NodeIdentity) -> RegistryResult<()> {
    validate_identifier("identity.node_id", &identity.node_id)
        .map_err(RegistryError::validation)?;
    for (field, value) in [
        ("identity.node_label", identity.node_label.as_str()),
        ("identity.hostname", identity.hostname.as_str()),
        ("identity.os", identity.os.as_str()),
        ("identity.agent_version", identity.agent_version.as_str()),
    ] {
        validate_bounded_text(field, value, MAX_NODE_IDENTITY_TEXT_BYTES)
            .map_err(RegistryError::validation)?;
    }
    for (field, value) in [
        (
            "identity.kernel_version",
            identity.kernel_version.as_deref(),
        ),
        ("identity.cpu_model", identity.cpu_model.as_deref()),
    ] {
        if let Some(value) = value {
            validate_bounded_text(field, value, MAX_NODE_IDENTITY_TEXT_BYTES)
                .map_err(RegistryError::validation)?;
        }
    }
    validate_tag_list(
        "identity.tags",
        &identity.tags,
        MAX_NODE_TAGS,
        MAX_NODE_TAG_BYTES,
    )
    .map_err(RegistryError::validation)?;
    Ok(())
}
