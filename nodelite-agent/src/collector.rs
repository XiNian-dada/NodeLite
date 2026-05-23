//! 主机指标采集器入口:按目标平台分派到具体实现。

#[cfg(target_os = "linux")]
#[path = "collector_linux.rs"]
mod collector_linux;
#[cfg(target_os = "linux")]
pub use collector_linux::{HostCollector, new_collector};

#[cfg(target_os = "macos")]
#[path = "collector_macos.rs"]
mod collector_macos;
#[cfg(target_os = "macos")]
pub use collector_macos::{HostCollector, new_collector};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "collector_unsupported.rs"]
mod collector_unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use collector_unsupported::{HostCollector, new_collector};
