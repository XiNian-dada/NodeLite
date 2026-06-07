use std::path::Path;

use chrono::{Duration as ChronoDuration, Utc};
use nodelite_proto::{
    MAX_NODE_TAG_BYTES, MAX_NODE_TAGS, normalize_string_list, validate_identifier,
    validate_non_empty, validate_tag_list,
};

use super::storage::mutate_registry_file;
use super::token::{
    generate_token, hash_token, mint_install_session, prune_expired_install_sessions,
};
use super::validate::validate_registered_node;
use super::{
    DEFAULT_TOKEN_VALIDITY_DAYS, IssueNodeRequest, IssueNodeResult, RegisteredNode, RegistryError,
    RegistryResult,
};

/// 创建或更新一个节点:首次出现时插入新条目,已存在时按需轮换 token、覆盖标签等。
///
/// 同时为该节点签发一个一次性安装令牌。这是 CLI 命令的核心入口。
pub async fn issue_node(path: &Path, request: IssueNodeRequest) -> RegistryResult<IssueNodeResult> {
    validate_identifier("node_id", &request.node_id).map_err(RegistryError::validation)?;
    if let Some(node_label) = request.node_label.as_deref() {
        validate_non_empty("node_label", node_label).map_err(RegistryError::validation)?;
    }
    let normalized_tags = normalize_string_list(request.tags.clone());
    validate_tag_list("tags", &normalized_tags, MAX_NODE_TAGS, MAX_NODE_TAG_BYTES)
        .map_err(RegistryError::validation)?;

    let request = request.clone();
    let (result, _) = mutate_registry_file(path, move |file| {
        let now = Utc::now();
        prune_expired_install_sessions(file, now);
        let mut rotated_token = false;

        if let Some(index) = file
            .nodes
            .iter()
            .position(|node| node.node_id == request.node_id)
        {
            if let Some(node_label) = request.node_label.as_ref() {
                file.nodes[index].node_label = node_label.trim().to_string();
            }
            if !request.tags.is_empty() {
                file.nodes[index].tags = normalized_tags.clone();
            }
            // install_session 必须带上当时有效的明文 token 给本次 install 流程使用;
            // #56 改造之后 disk 上不再有明文,因此唯一能把明文传给 agent 的位置就是这里。
            // 每次 issue_node 都强制轮换 token,确保一次 install 流程对应一次 token 颁发。
            let session_plaintext = {
                let new_token = generate_token()?;
                file.nodes[index].token_hash = hash_token(&new_token).map_err(|error| {
                    RegistryError::internal("failed to hash token", error.into())
                })?;
                file.nodes[index].token_generation =
                    file.nodes[index].token_generation.saturating_add(1);
                file.nodes[index].token_expires_at =
                    Some(now + ChronoDuration::days(DEFAULT_TOKEN_VALIDITY_DAYS));
                file.nodes[index].token.clear();
                rotated_token = true;
                new_token
            };

            validate_registered_node(&file.nodes[index])?;
            let node = file.nodes[index].clone();
            let install_session =
                mint_install_session(file, &node.node_id, now, session_plaintext.clone())?;
            return Ok((
                IssueNodeResult {
                    node,
                    node_session_token: session_plaintext,
                    created: false,
                    rotated_token,
                    install_token: install_session.token,
                    install_token_expires_at: install_session.expires_at,
                },
                true,
            ));
        }

        let plaintext_token = generate_token()?;
        let token_hash = hash_token(&plaintext_token).map_err(|error| {
            RegistryError::internal("failed to hash issued token", error.into())
        })?;
        let node = RegisteredNode {
            node_id: request.node_id.trim().to_string(),
            node_label: request
                .node_label
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(request.node_id.as_str())
                .to_string(),
            token_hash,
            token_generation: 1,
            token: String::new(),
            tags: normalized_tags.clone(),
            created_at: now,
            token_expires_at: Some(now + ChronoDuration::days(DEFAULT_TOKEN_VALIDITY_DAYS)),
        };
        validate_registered_node(&node)?;

        file.nodes.push(node.clone());
        file.nodes
            .sort_by(|left, right| left.node_id.cmp(&right.node_id));
        let install_session =
            mint_install_session(file, &node.node_id, now, plaintext_token.clone())?;
        Ok((
            IssueNodeResult {
                node,
                node_session_token: plaintext_token,
                created: true,
                rotated_token,
                install_token: install_session.token,
                install_token_expires_at: install_session.expires_at,
            },
            true,
        ))
    })
    .await?;

    Ok(result)
}
