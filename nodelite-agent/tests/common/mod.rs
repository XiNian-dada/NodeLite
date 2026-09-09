//! 集成测试共享辅助。
//!
//! 每个集成测试文件以 `mod common;` 引入;并非每个文件都会用到全部条目,
//! 因此放开 dead_code 警告。

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nodelite_proto::{AgentConfig, NodeIdentity};

/// RAII 临时目录:构造时创建唯一目录,析构时递归删除。
///
/// 相比"测试末尾手动 `remove_dir_all`"的写法,Drop 在 unwind(panic)时
/// 仍会执行,因此断言失败也不会泄漏临时目录。唯一性由进程 PID + 进程内
/// 原子计数器保证,无需读取系统时钟(从而避免 `SystemTime` 上的 unwrap)。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create unique temp dir for integration test");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 指向给定地址的 Agent 配置。`connect_timeout_secs` 取 2s,`report_interval_secs`
/// 取 5s,确保测试窗口内不会因为指标上报而产生额外流量。
pub fn test_config(local_addr: SocketAddr) -> AgentConfig {
    AgentConfig {
        node_id: "reconnect-node-01".to_string(),
        node_label: "Reconnect Node 01".to_string(),
        server: format!("ws://{local_addr}/ws"),
        token: "reconnect-token".to_string(),
        connect_timeout_secs: 2,
        auth_timeout_secs: 20,
        send_timeout_secs: 20,
        inbound_timeout_secs: 90,
        report_interval_secs: 5,
        max_incoming_message_bytes: 65536,
        insecure_transport_warn_interval_secs: 900,
        tags: vec![],
        hostname_override: None,
    }
}

pub fn test_identity(config: &AgentConfig) -> NodeIdentity {
    NodeIdentity {
        node_id: config.node_id.clone(),
        node_label: config.node_label.clone(),
        hostname: "localhost".to_string(),
        os: "test".to_string(),
        kernel_version: None,
        cpu_model: None,
        cpu_cores: 1,
        agent_version: "0.1.0-test".to_string(),
        boot_time: None,
        tags: vec![],
    }
}
