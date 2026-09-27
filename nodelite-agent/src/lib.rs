//! NodeLite 客户端核心库，负责主机指标采集、网络整形与 WebSocket 状态同步。

pub mod collector;
pub mod config_io;
pub mod runtime;
pub mod session;
pub(crate) mod traffic_control;
// `support` 只服务于本 crate 内部(runtime / session),不属于对外测试 API,
// 因此收敛为 pub(crate),避免把内部辅助函数暴露给库的下游使用者。
pub(crate) mod support;
