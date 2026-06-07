//! macOS host identity collection and plist parsing.

use std::fs;

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};
use nodelite_proto::{AgentConfig, NodeIdentity};
use tracing::warn;

use super::syscall;

pub(super) fn collect_identity(config: &AgentConfig, agent_version: &str) -> Result<NodeIdentity> {
    let uptime_secs = syscall::read_uptime_secs()?;
    let boot_time = Utc::now() - Duration::seconds(i64::try_from(uptime_secs).unwrap_or(i64::MAX));

    Ok(NodeIdentity {
        node_id: config.node_id.clone(),
        node_label: config.node_label.clone(),
        hostname: config.hostname_override.clone().unwrap_or(read_hostname()?),
        os: read_os_name().unwrap_or_else(|error| {
            warn!(
                error = ?error,
                "failed to parse macOS SystemVersion.plist; falling back to generic OS name"
            );
            "macOS".to_string()
        }),
        kernel_version: read_kernel_version().ok(),
        cpu_model: read_cpu_model().ok(),
        cpu_cores: syscall::count_cpu_cores().unwrap_or(1),
        agent_version: agent_version.to_string(),
        boot_time: Some(boot_time),
        tags: config.tags.clone(),
    })
}

fn read_hostname() -> Result<String> {
    let uts = syscall::read_uname()?;
    syscall::c_chars_to_string(&uts.nodename)
}

fn read_kernel_version() -> Result<String> {
    let uts = syscall::read_uname()?;
    syscall::c_chars_to_string(&uts.release)
}

/// 解析系统版本 plist,优先输出 `ProductName ProductVersion`。
fn read_os_name() -> Result<String> {
    let content = fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .context("read /System/Library/CoreServices/SystemVersion.plist")?;
    parse_os_name_from_plist(&content)
}

pub(super) fn parse_os_name_from_plist(content: &str) -> Result<String> {
    let product_name = extract_plist_value(content, "ProductName");
    let product_version = extract_plist_value(content, "ProductVersion");

    match (product_name, product_version) {
        (Some(name), Some(version)) => Ok(format!("{name} {version}")),
        (Some(name), None) => Ok(name),
        (None, Some(version)) => Ok(format!("macOS {version}")),
        (None, None) => Err(anyhow!(
            "ProductName/ProductVersion missing from SystemVersion.plist"
        )),
    }
}

/// 仅解析 Apple `SystemVersion.plist` 里 `<key>` 后紧邻 `<string>` 的简单模式。
/// 如果 plist 结构改成注释、CDATA 或其它元素穿插,这里会返回 `None` 让调用方走告警回退。
pub(super) fn extract_plist_value(content: &str, key: &str) -> Option<String> {
    let key_marker = format!("<key>{key}</key>");
    let key_index = content.find(&key_marker)?;
    let remainder = &content[key_index + key_marker.len()..];
    let string_start = remainder.find("<string>")?;
    let remainder = &remainder[string_start + "<string>".len()..];
    let string_end = remainder.find("</string>")?;
    Some(remainder[..string_end].trim().to_string())
}

fn read_cpu_model() -> Result<String> {
    syscall::read_sysctl_string(b"machdep.cpu.brand_string\0")
        .or_else(|_| syscall::read_sysctl_string(b"hw.model\0"))
}
