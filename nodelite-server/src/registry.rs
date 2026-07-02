//! 节点注册表:服务端唯一的"哪些节点被允许上报"的事实来源。
//!
//! 注册表是一份 JSON 文件,内容由 `RegistryFile` 结构序列化得到。
//! 服务端进程与运维 CLI(`server install-agent` 等)都会读写这份文件,
//! 因此对每次写入都采用 flock + 原子替换的策略。
//!
//! 字段语义:
//! - [`RegisteredNode`]:被认证的 Agent 凭证(node_id + token)。
//! - [`InstallSession`]:一次性的"安装令牌",拥有它可以拉取 Agent 配置。

mod auth;
mod error;
mod issue;
mod lifecycle;
mod render;
mod storage;
#[cfg(test)]
mod tests;
mod token;
mod validate;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use lru::LruCache;
use nodelite_proto::{GeoIpLocation, NodeIdentity};
use parking_lot::Mutex as ParkingLotMutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, Semaphore};

#[cfg(test)]
use self::auth::TokenVerifyProbe;
pub use self::error::{RegistryError, RegistryResult};
pub use self::issue::issue_node;
#[cfg(test)]
pub use self::render::{build_agent_server_url, build_github_release_base_url};
pub use self::render::{
    build_install_script_url, default_agent_release_base_url, render_agent_config,
    render_install_command, render_upgrade_command,
};
#[cfg(test)]
use self::storage::{
    MAX_REGISTRY_FILE_BYTES, registry_file_read_count, release_registry_lock_with,
    reset_registry_file_read_count,
};
#[cfg(test)]
use self::token::{generate_token, token_is_unexpired, verify_token};
#[cfg(test)]
use self::validate::validate_registered_node;
#[cfg(test)]
use nodelite_proto::{MAX_NODE_TAG_BYTES, normalize_string_list};

/// Agent Token 默认有效期:30 天。
const DEFAULT_TOKEN_VALIDITY_DAYS: i64 = 30;

/// Argon2id 参数:用 OWASP 2023 推荐的"低延迟服务器"档位,大约 ~12-25ms/verify。
/// memory=19 MiB, iterations=2, parallelism=1。
/// 落在 WS Hello 等"每会话一次"的路径上是可以接受的,但 #56 的设计要求 hot-path
/// 通过 `token_generation` 比较而非每次 verify。
const ARGON2_MEMORY_KIB: u32 = 19 * 1024;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;

/// 一次性安装令牌的有效期(分钟)。
const INSTALL_TOKEN_TTL_MINUTES: i64 = 15;
/// Argon2id 每次 verify 会短时占用约 19MiB 内存。从 2 提升至 8 以改善
/// 重连风暴场景的排队延迟,同时内存峰值仍可控(8 × 19MiB = 152MiB)。
/// 实际运行时会被 `available_parallelism().min(8)` 限制。
/// TODO: 后续可考虑配置化,允许低内存 VPS 降至 2 或 4。
const TOKEN_VERIFY_MAX_PARALLELISM: usize = 8;
const TOKEN_VERIFY_WAIT_WARN_AFTER: Duration = Duration::from_millis(100);
const LOCATION_COORDINATE_SCALE: f64 = 1_000_000.0;

/// Token 验证结果缓存容量:支持最多 500 个节点的重连场景。
/// 配合 8 并发度的 Argon2id verify,200 节点 reconnect storm 实测:
/// p50 从 640ms 降至 ~70ms,p95 从 1,689ms 降至 ~700ms。
///
/// 注意:实测 cache hit rate 约 33%,低于理论值(200 nodes 放入 512 容量应该
/// 无 eviction)。可能原因:并发 miss、registry_revision 变化、或统计口径问题。
/// TODO: 添加 cache_hit/miss/eviction 计数以诊断实际缓存效率。
/// TODO: 后续可考虑配置化,允许低内存 VPS 降至 128 或 256。
const TOKEN_CACHE_CAPACITY: usize = 512;
/// Token 验证结果缓存 TTL:5 分钟。重连场景下可直接命中缓存,避免 Argon2id 开销。
const TOKEN_CACHE_TTL: Duration = Duration::from_secs(300);

/// 已登记节点的持久化条目。
///
/// Token 存储语义 (#56):
/// - 新条目 **不再** 在 `token` 字段保留明文; 只保留 `token_hash` 的 Argon2id PHC 字符串。
/// - `token_generation` 每次 token 轮换递增一次, 供 WS hot-path
///   `is_token_current` 做 O(1) 比较而不必每条消息都跑 Argon2 verify。
/// - `token` 字段保留是为了 **向后兼容**: 老版本写出的 registry.json 仍然
///   能被读取; `load_registry_state` 会在首次加载时把 `token` 哈希并清空,
///   随即把升级后的文件写回磁盘 —— 之后磁盘上不再出现明文。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredNode {
    pub node_id: String,
    pub node_label: String,
    /// Argon2id PHC 编码的 token 哈希。空串表示 "尚未迁移过的旧条目",
    /// 此时应当用 `token` 字段做最后一次明文比较并触发迁移。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_hash: String,
    /// 单调递增的 token 代次。每次 `refresh_token` / `issue_node` 轮换都 +1。
    /// WS 会话在认证时捕获这个值, 后续 hot-path 只比较代次,
    /// 避免每条消息都跑 Argon2 verify。
    #[serde(default)]
    pub token_generation: u64,
    /// Legacy: 旧版本的明文 token 字段。新版本启动后会一次性把它哈希到
    /// `token_hash` 并清空, 之后磁盘上不再出现。保留 #[serde(default)]
    /// 与 skip_serializing_if 是为了让升级与降级都能干净通过。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    /// Token 过期时间。None 表示永不过期(向后兼容旧版本注册表)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<DateTime<Utc>>,
    /// 运营侧记录的服务器/套餐到期时间,仅用于设置页展示与提醒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_expires_at: Option<DateTime<Utc>>,
    /// 运营侧标记的自持有/无限期服务器,为 true 时不展示到期倒计时。
    #[serde(default, skip_serializing_if = "is_false")]
    pub service_unlimited: bool,
    /// 运营侧记录的续费价格,例如 "¥30/月" 或 "$5/mo"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renewal_price: Option<String>,
    /// 手动位置覆盖。设置后 UI 地图与位置文本优先使用这里的值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_latitude_microdegrees: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_longitude_microdegrees: Option<i32>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 一次成功的 token 验证 / 颁发结果:返回身份与 token 状态快照,
/// 供 WS 会话缓存后避开每帧 registry 读锁。
#[derive(Debug, Clone)]
pub struct AuthorizedNode {
    pub identity: NodeIdentity,
    pub generation: u64,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub registry_revision: u64,
    pub location_override: Option<GeoIpLocation>,
}

/// 轻量 token 状态快照,供 WebSocket 会话在 registry revision 变化时刷新本地缓存。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryTokenStatus {
    pub generation: u64,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub registry_revision: u64,
}

/// `consume_install_token` 的成功返回值:Agent 拿到这个结构后即可写出本地配置。
#[derive(Debug, Clone)]
pub struct ConsumedInstall {
    pub node: RegisteredNode,
    /// 节点的明文 session token, 由 install_session 短暂持有, 返回后立即从注册表删除。
    pub node_session_token: String,
}

/// 安装会话:由 CLI 颁发的一次性令牌,Agent 用它拉取自己的配置。
///
/// `expires_at` 为绝对过期时间;每次写入注册表时会顺带清理已过期会话。
///
/// Token 存储 (#56):
/// - `node_session_token` 持有该 install_session 所属节点的**明文 session token**。
///   这是 Argon2id 化之后整个系统里唯一暂存明文的位置, 生存周期 <= 15 分钟
///   (`INSTALL_TOKEN_TTL_MINUTES`), 一旦 `consume_install_token` 被调用就连同
///   整条 session 一起从注册表删除。比起 #56 之前的 "永久保留 token 明文"
///   是一个明显的硬化。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallSession {
    pub token: String,
    pub node_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// 节点的明文 session token, 仅在 install 流程消费之前短暂持有。
    /// 老版本的 install_session 不带这个字段,旧的 install_token 在升级后
    /// 自然过期(15min)、不可恢复 —— 运维需要重新颁发 install_token。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_session_token: String,
}

/// Token 验证缓存键:(token_hash_hex, registry_revision)。
/// 使用 token 的 SHA256 哈希作为缓存键,避免存储明文。
type TokenCacheKey = (String, u64);

/// Token 验证缓存值:(验证结果, 缓存时间戳)。
struct TokenCacheEntry {
    verified: bool,
    cached_at: Instant,
}

/// 注册表的运行期视图:进程内部以 HashMap 形式持有,便于鉴权 / 查询。
#[derive(Debug, Clone)]
pub struct NodeRegistry {
    path: Arc<PathBuf>,
    state: Arc<RwLock<RegistryState>>,
    reload_checkpoint: Arc<RwLock<RegistryReloadCheckpoint>>,
    registry_revision: Arc<AtomicU64>,
    token_verify_limit: usize,
    token_verify_limiter: Arc<Semaphore>,
    /// Token 验证结果缓存:减少重连场景的 Argon2id 开销。
    token_cache: Arc<ParkingLotMutex<LruCache<TokenCacheKey, TokenCacheEntry>>>,
    #[cfg(test)]
    token_verify_probe: Option<Arc<TokenVerifyProbe>>,
}

#[derive(Debug)]
struct RegistryReloadCheckpoint {
    fingerprint: Option<storage::RegistryFileFingerprint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RegistryState {
    entries: HashMap<String, RegisteredNode>,
    install_sessions: HashMap<String, InstallSession>,
}

/// `server install-agent` / `server issue-node` 等命令传给注册表的请求结构。
#[derive(Debug, Clone)]
pub struct IssueNodeRequest {
    pub node_id: String,
    pub node_label: Option<String>,
    pub tags: Vec<String>,
}

/// `IssueNodeRequest` 的结果集:同时返回节点凭证与一次性安装令牌。
#[derive(Debug, Clone)]
pub struct IssueNodeResult {
    pub node: RegisteredNode,
    pub node_session_token: String,
    pub created: bool,
    pub rotated_token: bool,
    pub install_token: String,
    pub install_token_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    #[serde(default)]
    version: u64,
    #[serde(default)]
    nodes: Vec<RegisteredNode>,
    #[serde(default)]
    install_sessions: Vec<InstallSession>,
}

impl RegisteredNode {
    pub fn location_override(&self) -> Option<GeoIpLocation> {
        let country = self.location_override_country.clone()?;
        Some(GeoIpLocation {
            country,
            city: self.location_override_city.clone(),
            latitude: self
                .location_override_latitude_microdegrees
                .map(microdegrees_to_coordinate),
            longitude: self
                .location_override_longitude_microdegrees
                .map(microdegrees_to_coordinate),
        })
    }
}

pub(super) fn coordinate_to_microdegrees(value: f64) -> i32 {
    (value * LOCATION_COORDINATE_SCALE).round() as i32
}

fn microdegrees_to_coordinate(value: i32) -> f64 {
    f64::from(value) / LOCATION_COORDINATE_SCALE
}
