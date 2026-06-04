use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use tokio::runtime::Runtime;

use super::{
    IssueNodeRequest, MAX_NODE_TAG_BYTES, NodeRegistry, RegisteredNode, RegistryError,
    RegistryFile, TokenVerifyProbe, build_agent_server_url, build_github_release_base_url,
    default_agent_release_base_url, issue_node, registry_file_read_count,
    release_registry_lock_with, render_install_command, reset_registry_file_read_count,
    token_is_unexpired, validate_registered_node, verify_token,
};
use nodelite_proto::NodeIdentity;

mod authorize_tests;
mod error_tests;
mod migrate_tests;
mod reload_tests;
mod store_tests;
mod token_tests;
mod validate_tests;

fn legacy_node(
    node_id: &str,
    node_label: &str,
    token: &str,
    token_expires_at: Option<DateTime<Utc>>,
) -> RegisteredNode {
    RegisteredNode {
        node_id: node_id.to_string(),
        node_label: node_label.to_string(),
        token_hash: String::new(),
        token_generation: 0,
        token: token.to_string(),
        tags: Vec::new(),
        created_at: Utc::now(),
        token_expires_at,
    }
}

fn identity_for(node_id: &str) -> NodeIdentity {
    NodeIdentity {
        node_id: node_id.to_string(),
        node_label: node_id.to_string(),
        hostname: format!("{node_id}.internal"),
        os: "Ubuntu".to_string(),
        kernel_version: None,
        cpu_model: None,
        cpu_cores: 2,
        agent_version: "0.1.0".to_string(),
        boot_time: None,
        tags: Vec::new(),
    }
}
