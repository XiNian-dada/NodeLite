//! 安全审计日志模块结构与公共导出。

#[cfg(test)]
mod disabled_tests;
mod log;
mod query;
mod storage;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod support;
#[cfg(test)]
mod tests;
mod types;
mod writer;

pub(crate) use self::log::AuditLog;
pub use self::types::{AuditEvent, AuditEventType, AuditLogError, AuditQuery, NewAuditEvent};
