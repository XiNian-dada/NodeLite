//! 非 Linux / macOS 平台的占位实现:保持类型与函数签名一致,
//! 运行期返回清晰错误,避免 `cargo build` 之类的开发流程直接失败。

use anyhow::{Result, anyhow};
use nodelite_proto::{AgentConfig, NodeIdentity, NodeSnapshot};

pub struct HostCollector;

pub fn new_collector() -> HostCollector {
    HostCollector
}

impl HostCollector {
    pub fn collect_identity(
        &self,
        _config: &AgentConfig,
        _agent_version: &str,
    ) -> Result<NodeIdentity> {
        Err(anyhow!(
            "nodelite-agent only supports Linux and macOS targets"
        ))
    }

    pub fn collect_snapshot(&mut self) -> Result<NodeSnapshot> {
        Err(anyhow!(
            "nodelite-agent only supports Linux and macOS targets"
        ))
    }
}
