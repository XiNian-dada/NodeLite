//! 配置与注册表共用的轻量校验工具。
//!
//! 这些规则同时服务于:
//! - `config.rs` 对 TOML 配置的解析校验;
//! - `registry.rs` 对节点 / install session / runtime identity 的约束检查。
//!
//! 统一放在这里,可以避免两处实现渐渐漂移。

use std::fmt;

/// 配置与注册表共享校验工具返回的校验失败错误。
///
/// 错误消息设计为可直接对外展示，方便在配置解析报错或 HTTP 接口中向用户返回具体原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    /// 构造包含可读错误信息的校验错误。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

const IDENTIFIER_MAX_CHARS: usize = 128;

/// 拒绝在剔除前后 ASCII 与 Unicode 空白后为空的输入值。
///
/// `field` 字段名原样包含在返回的错误中，便于调用方定位具体的 TOML 键、JSON 属性或注册表字段。
pub fn validate_non_empty(field: &str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        return Err(ValidationError::new(format!("{field} must not be empty")));
    }
    Ok(())
}

/// 校验非空展示/协议文本，限制最大 UTF-8 字节数并拒绝控制字符。
pub fn validate_bounded_text(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ValidationError> {
    validate_non_empty(field, value)?;
    if value.len() > max_bytes {
        return Err(ValidationError::new(format!(
            "{field} must be <= {max_bytes} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(())
}

/// 校验稳定的 NodeLite 标识符（如节点 ID、组名等）。
///
/// 标识符必须非空、长度不超过 128 字符，且限定为 ASCII 字母、数字、连字符、下划线与点号。
/// 确保其在日志输出、文件系统路径与指标 Label 中安全无歧义。
pub fn validate_identifier(field: &str, value: &str) -> Result<(), ValidationError> {
    validate_non_empty(field, value)?;
    if value.len() > IDENTIFIER_MAX_CHARS {
        return Err(ValidationError::new(format!(
            "{field} must be <= {IDENTIFIER_MAX_CHARS} characters"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ValidationError::new(format!(
            "{field} must use only ASCII letters, numbers, '-', '_' or '.'"
        )));
    }
    Ok(())
}

/// 修剪、排序并去重运维人员提供的字符串列表。
///
/// 过滤空白条目，输出结果保持稳定有序，确保配置序列化往返和单元测试的可重现性。
pub fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

/// 校验已归一化的节点标签列表，限制标签数量和单个标签字节上限。
///
/// 本函数不负责修剪或去重；针对自由格式输入请先调用 [`normalize_string_list`]。
/// `max_tag_bytes` 使用 `String::len` 计算，为 UTF-8 字节数限制而非字符数。
pub fn validate_tag_list(
    field: &str,
    values: &[String],
    max_tags: usize,
    max_tag_bytes: usize,
) -> Result<(), ValidationError> {
    if values.len() > max_tags {
        return Err(ValidationError::new(format!(
            "{field} must contain at most {max_tags} tags"
        )));
    }
    for (index, value) in values.iter().enumerate() {
        validate_bounded_text(&format!("{field}[{index}]"), value, max_tag_bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ValidationError, normalize_string_list, validate_bounded_text, validate_identifier,
        validate_non_empty, validate_tag_list,
    };

    #[test]
    fn validation_error_displays_original_message() {
        let error = ValidationError::new("boom");
        assert_eq!(error.to_string(), "boom");
    }

    #[test]
    fn normalize_string_list_trims_sorts_and_deduplicates() {
        let values = normalize_string_list(vec![
            " beta ".to_string(),
            "".to_string(),
            "alpha".to_string(),
            "beta".to_string(),
        ]);
        assert_eq!(values, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn validation_helpers_reject_invalid_values() {
        let error = validate_non_empty("name", "   ").expect_err("blank values should fail");
        assert_eq!(error.to_string(), "name must not be empty");

        let error =
            validate_identifier("node_id", "bad value").expect_err("spaces are not allowed");
        assert!(error.to_string().contains("ASCII letters"));

        let error = validate_tag_list("tags", &[String::from("abcdef")], 4, 5)
            .expect_err("oversized tags should fail");
        assert_eq!(error.to_string(), "tags[0] must be <= 5 bytes");

        let error = validate_bounded_text("label", "bad\nlabel", 32)
            .expect_err("control characters should fail");
        assert_eq!(
            error.to_string(),
            "label must not contain control characters"
        );
    }

    #[test]
    fn validation_helpers_accept_expected_happy_paths() {
        validate_non_empty("name", "node-lite").expect("non-empty values should pass");
        validate_bounded_text("label", "节点", 6).expect("UTF-8 byte boundary should pass");
        validate_identifier("node_id", "hk-01.edge_1").expect("valid identifiers should pass");
        validate_tag_list("tags", &[String::from("edge"), String::from("prod")], 4, 8)
            .expect("small tag lists should pass");
    }

    #[test]
    fn validate_identifier_rejects_values_longer_than_limit() {
        let too_long = "a".repeat(129);
        let error = validate_identifier("node_id", &too_long)
            .expect_err("overlong identifiers should fail");
        assert_eq!(error.to_string(), "node_id must be <= 128 characters");
    }

    #[test]
    fn validate_tag_list_rejects_too_many_values() {
        let error = validate_tag_list(
            "tags",
            &[
                String::from("edge"),
                String::from("prod"),
                String::from("cn"),
            ],
            2,
            8,
        )
        .expect_err("too many tags should fail");
        assert_eq!(error.to_string(), "tags must contain at most 2 tags");
    }
}
