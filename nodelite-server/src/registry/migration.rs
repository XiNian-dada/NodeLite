//! 旧注册表中展示元数据的一次性安全迁移。

use nodelite_proto::{
    MAX_NODE_IDENTITY_TEXT_BYTES, MAX_NODE_TAG_BYTES, MAX_NODE_TAGS, normalize_string_list,
    truncate_string_to_byte_boundary,
};

use super::RegistryFile;

pub(super) fn migrate_legacy_display_metadata(file: &mut RegistryFile) -> bool {
    let mut changed = false;
    for node in &mut file.nodes {
        let mut node_label =
            sanitize_legacy_text(node.node_label.as_str(), MAX_NODE_IDENTITY_TEXT_BYTES);
        if node_label.is_empty() {
            node_label = node.node_id.clone();
        }
        if node.node_label != node_label {
            node.node_label = node_label;
            changed = true;
        }

        let mut tags = normalize_string_list(
            node.tags
                .iter()
                .map(|tag| sanitize_legacy_text(tag, MAX_NODE_TAG_BYTES))
                .filter(|tag| !tag.is_empty())
                .collect(),
        );
        tags.truncate(MAX_NODE_TAGS);
        if node.tags != tags {
            node.tags = tags;
            changed = true;
        }
    }
    changed
}

fn sanitize_legacy_text(value: &str, max_bytes: usize) -> String {
    let mut sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    sanitized = sanitized.trim().to_string();
    truncate_string_to_byte_boundary(&mut sanitized, max_bytes);
    sanitized.trim_end().to_string()
}
