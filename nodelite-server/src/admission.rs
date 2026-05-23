//! WebSocket 与 install 端点的准入控制。
//!
//! 三件事在一起:
//! 1. [`WsAdmissionController`] 对 `/ws` 做"总量 + 单 IP"并发限流,叠加认证失败封禁。
//! 2. [`InstallAdmissionController`] 仅按 IP 做认证失败封禁,因为 install 是
//!    一次性短请求,没有"活动连接数"概念,被复用为 `/api/verify-2fa` 的 IP 限流。
//! 3. [`resolve_client_ip`] 在反向代理场景下解析真实客户端 IP,以及 [`AuthFailureState`]
//!    及其 prune/sweep helpers 这两个 controller 共享。

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ipnet::IpNet;
use nodelite_proto::WsConfig;

/// 软上限:超过后顺手扫一次表清掉已过期条目。攻击者用大量伪造 IP 制造
/// 一次性失败时,本表只在攻击侧累积代价,稳态体积可控。
const WS_AUTH_FAILURE_TABLE_SOFT_LIMIT: usize = 1024;
const INSTALL_AUTH_FAILURE_TABLE_SOFT_LIMIT: usize = 1024;
/// 硬上限:即便攻击者持续制造仍在窗口内的失败 IP,失败表也不能无界增长。
const WS_AUTH_FAILURE_TABLE_HARD_LIMIT: usize = 4096;
const INSTALL_AUTH_FAILURE_TABLE_HARD_LIMIT: usize = 4096;

/// 单个 IP 的认证失败历史。
#[derive(Debug, Default)]
pub struct AuthFailureState {
    pub recent_failures: VecDeque<Instant>,
    pub blocked_until: Option<Instant>,
}

pub fn auth_failure_admission_config(ws: &WsConfig) -> InstallAdmissionConfig {
    InstallAdmissionConfig {
        auth_fail_window_secs: ws.auth_fail_window_secs,
        auth_fail_max_attempts: ws.auth_fail_max_attempts,
        auth_block_secs: ws.auth_block_secs,
    }
}

/// 敏感写操作沿用同一失败窗口与封禁时长,但尝试预算更低:
/// 例如密码修改、2FA 开关、服务端自更新这些路径不应允许和只读看板一样多的
/// 猜测机会。
pub fn sensitive_auth_failure_admission_config(ws: &WsConfig) -> InstallAdmissionConfig {
    InstallAdmissionConfig {
        auth_fail_window_secs: ws.auth_fail_window_secs,
        auth_fail_max_attempts: (ws.auth_fail_max_attempts / 2).max(1),
        auth_block_secs: ws.auth_block_secs,
    }
}

/// WebSocket 准入控制器:封装总量限流、IP 限流与认证失败封禁。
#[derive(Clone)]
pub struct WsAdmissionController {
    config: WsConfig,
    state: Arc<Mutex<WsAdmissionState>>,
}

#[derive(Debug, Default)]
struct WsAdmissionState {
    total_active_connections: usize,
    active_by_ip: HashMap<IpAddr, usize>,
    auth_failures: HashMap<IpAddr, AuthFailureState>,
}

/// RAII 句柄:存在意味着占用一个 WebSocket 连接槽,析构时归还。
pub struct WsConnectionPermit {
    controller: WsAdmissionController,
    client_ip: IpAddr,
}

/// 准入失败的具体原因,对应到不同的 HTTP 响应。
#[derive(Debug)]
pub enum WsAdmissionError {
    TotalCapacity,
    IpCapacity,
    Blocked { retry_after_secs: u64 },
}

impl WsAdmissionController {
    pub fn new(config: &WsConfig) -> Self {
        Self {
            config: config.clone(),
            state: Arc::new(Mutex::new(WsAdmissionState::default())),
        }
    }

    /// 尝试占用一个 WebSocket 连接配额。
    ///
    /// 返回 RAII 句柄;它一旦析构,连接计数会被自动回退,无需手动 release。
    pub fn try_acquire(&self, client_ip: IpAddr) -> Result<WsConnectionPermit, WsAdmissionError> {
        let now = Instant::now();
        let mut state = self.lock_state();
        let failure_window = Duration::from_secs(self.config.auth_fail_window_secs);
        let failure_state = state.auth_failures.entry(client_ip).or_default();
        prune_auth_failure_state(failure_state, now, failure_window);
        if let Some(blocked_until) = failure_state.blocked_until
            && blocked_until > now
        {
            return Err(WsAdmissionError::Blocked {
                retry_after_secs: blocked_until.duration_since(now).as_secs().max(1),
            });
        }
        if failure_state.recent_failures.is_empty() && failure_state.blocked_until.is_none() {
            state.auth_failures.remove(&client_ip);
        }

        if state.total_active_connections >= self.config.max_total_connections {
            return Err(WsAdmissionError::TotalCapacity);
        }
        let active_for_ip = state.active_by_ip.get(&client_ip).copied().unwrap_or(0);
        if active_for_ip >= self.config.max_connections_per_ip {
            return Err(WsAdmissionError::IpCapacity);
        }

        state.total_active_connections = state.total_active_connections.saturating_add(1);
        state.active_by_ip.insert(client_ip, active_for_ip + 1);

        Ok(WsConnectionPermit {
            controller: self.clone(),
            client_ip,
        })
    }

    /// 记录一次认证失败,达到阈值后把客户端 IP 临时封禁。
    pub fn record_auth_failure(&self, client_ip: IpAddr) {
        let now = Instant::now();
        let failure_window = Duration::from_secs(self.config.auth_fail_window_secs);
        let mut state = self.lock_state();
        let failure_state = state.auth_failures.entry(client_ip).or_default();
        prune_auth_failure_state(failure_state, now, failure_window);
        failure_state.recent_failures.push_back(now);
        if failure_state.recent_failures.len() >= self.config.auth_fail_max_attempts {
            failure_state.blocked_until =
                Some(now + Duration::from_secs(self.config.auth_block_secs));
            failure_state.recent_failures.clear();
        }
        // 攻击者可以用一波伪造 IP 把失败表撑大,而单 IP 后续不再回访就不会被
        // `try_acquire` / `prune_auth_failure_state` 主动清理 —— 在长跑实例里
        // 这是一条慢速内存泄漏。表过大时顺手做一次全表扫描,把已过期且未封禁
        // 的条目删掉;代价 O(N) 但摊销到攻击侧,稳态查询路径仍然 O(1)。
        trim_auth_failure_table(
            &mut state.auth_failures,
            now,
            failure_window,
            WS_AUTH_FAILURE_TABLE_SOFT_LIMIT,
            WS_AUTH_FAILURE_TABLE_HARD_LIMIT,
        );
    }

    /// 认证成功后清理该 IP 的失败历史。
    pub fn clear_auth_failures(&self, client_ip: IpAddr) {
        let mut state = self.lock_state();
        state.auth_failures.remove(&client_ip);
    }

    /// 由 `WsConnectionPermit::drop` 调用,把计数减回去。
    fn release_connection(&self, client_ip: IpAddr) {
        let mut state = self.lock_state();
        state.total_active_connections = state.total_active_connections.saturating_sub(1);
        if let Some(active_for_ip) = state.active_by_ip.get_mut(&client_ip) {
            *active_for_ip = active_for_ip.saturating_sub(1);
            if *active_for_ip == 0 {
                state.active_by_ip.remove(&client_ip);
            }
        }
    }

    /// 锁住状态;锁中毒说明上一次持锁路径发生 panic,此时保留有效状态,只清理过期条目。
    fn lock_state(&self) -> std::sync::MutexGuard<'_, WsAdmissionState> {
        self.state.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("WsAdmissionController mutex poisoned; preserving valid state, pruning expired entries");
            let mut guard = poisoned.into_inner();
            let now = Instant::now();
            let failure_window = Duration::from_secs(self.config.auth_fail_window_secs);
            sweep_expired_auth_failures(&mut guard.auth_failures, now, failure_window);
            guard.auth_failures.retain(|_, failure_state| {
                failure_state.blocked_until.is_some_and(|until| until > now)
                    || !failure_state.recent_failures.is_empty()
            });
            enforce_auth_failure_hard_cap(
                &mut guard.auth_failures,
                WS_AUTH_FAILURE_TABLE_HARD_LIMIT,
            );
            reconcile_ws_connection_counters(&mut guard, &self.config);
            self.state.clear_poison();
            guard
        })
    }
}

impl Drop for WsConnectionPermit {
    fn drop(&mut self) {
        self.controller.release_connection(self.client_ip);
    }
}

fn reconcile_ws_connection_counters(state: &mut WsAdmissionState, config: &WsConfig) {
    state.active_by_ip.retain(|_, active| *active > 0);
    let summed_connections = state
        .active_by_ip
        .values()
        .copied()
        .fold(0usize, usize::saturating_add);
    state.total_active_connections = state
        .total_active_connections
        .max(summed_connections)
        .min(config.max_total_connections);
}

/// `/install/bootstrap` 与 `/api/verify-2fa` 共用的 IP 维度认证失败计数器,
/// 与 `WsAdmissionController` 的封禁逻辑完全同型,但不维护"活动连接数":
/// 这些路径是一次性短请求,只需要按 IP 限制无效尝试,防止远端攻击者把
/// registry flock + IO 或 TOTP 校验当作免费 DoS 通道。
#[derive(Clone)]
pub struct InstallAdmissionController {
    config: InstallAdmissionConfig,
    state: Arc<Mutex<InstallAdmissionState>>,
}

#[derive(Debug, Clone, Copy)]
pub struct InstallAdmissionConfig {
    pub auth_fail_window_secs: u64,
    pub auth_fail_max_attempts: usize,
    pub auth_block_secs: u64,
}

#[derive(Debug, Default)]
struct InstallAdmissionState {
    auth_failures: HashMap<IpAddr, AuthFailureState>,
}

impl InstallAdmissionController {
    pub fn new(config: InstallAdmissionConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(InstallAdmissionState::default())),
        }
    }

    /// 检查给定 client_ip 是否仍处于封禁窗口。封禁中返回 `retry_after_secs`;
    /// 否则正常放行(不递增计数 —— `record_auth_failure` 才会递增)。
    pub fn check(&self, client_ip: IpAddr) -> Result<(), u64> {
        let now = Instant::now();
        let mut state = self.lock_state();
        let failure_window = Duration::from_secs(self.config.auth_fail_window_secs);
        let failure_state = state.auth_failures.entry(client_ip).or_default();
        prune_auth_failure_state(failure_state, now, failure_window);
        if let Some(blocked_until) = failure_state.blocked_until
            && blocked_until > now
        {
            return Err(blocked_until.duration_since(now).as_secs().max(1));
        }
        if failure_state.recent_failures.is_empty() && failure_state.blocked_until.is_none() {
            state.auth_failures.remove(&client_ip);
        }
        Ok(())
    }

    pub fn record_auth_failure(&self, client_ip: IpAddr) {
        let now = Instant::now();
        let failure_window = Duration::from_secs(self.config.auth_fail_window_secs);
        let mut state = self.lock_state();
        let failure_state = state.auth_failures.entry(client_ip).or_default();
        prune_auth_failure_state(failure_state, now, failure_window);
        failure_state.recent_failures.push_back(now);
        if failure_state.recent_failures.len() >= self.config.auth_fail_max_attempts {
            failure_state.blocked_until =
                Some(now + Duration::from_secs(self.config.auth_block_secs));
            failure_state.recent_failures.clear();
        }
        trim_auth_failure_table(
            &mut state.auth_failures,
            now,
            failure_window,
            INSTALL_AUTH_FAILURE_TABLE_SOFT_LIMIT,
            INSTALL_AUTH_FAILURE_TABLE_HARD_LIMIT,
        );
    }

    pub fn clear_auth_failures(&self, client_ip: IpAddr) {
        let mut state = self.lock_state();
        state.auth_failures.remove(&client_ip);
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, InstallAdmissionState> {
        self.state.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("InstallAdmissionController mutex poisoned; preserving valid state, pruning expired entries");
            let mut guard = poisoned.into_inner();
            let now = Instant::now();
            let failure_window = Duration::from_secs(self.config.auth_fail_window_secs);
            sweep_expired_auth_failures(&mut guard.auth_failures, now, failure_window);
            guard.auth_failures.retain(|_, failure_state| {
                failure_state.blocked_until.is_some_and(|until| until > now)
                    || !failure_state.recent_failures.is_empty()
            });
            enforce_auth_failure_hard_cap(
                &mut guard.auth_failures,
                INSTALL_AUTH_FAILURE_TABLE_HARD_LIMIT,
            );
            self.state.clear_poison();
            guard
        })
    }
}

/// 解析客户端真实 IP。
///
/// 仅当 TCP 直连对端属于可信代理(默认含 loopback)时,才会解析
/// `X-Forwarded-For` / `X-Real-IP`;其他直连请求一律使用 socket peer IP,
/// 避免公网客户端伪造代理头。
pub fn resolve_client_ip(
    trusted_proxies: &[IpNet],
    peer_addr: SocketAddr,
    headers: &HeaderMap,
) -> IpAddr {
    if !ip_is_trusted_proxy(peer_addr.ip(), trusted_proxies) {
        return peer_addr.ip();
    }

    forwarded_ip_from_headers(peer_addr.ip(), trusted_proxies, headers)
        .unwrap_or_else(|| peer_addr.ip())
}

fn ip_is_trusted_proxy(ip: IpAddr, trusted_proxies: &[IpNet]) -> bool {
    ip.is_loopback() || trusted_proxies.iter().any(|network| network.contains(&ip))
}

fn forwarded_ip_from_headers(
    peer_ip: IpAddr,
    trusted_proxies: &[IpNet],
    headers: &HeaderMap,
) -> Option<IpAddr> {
    forwarded_chain_from_header(headers)
        .and_then(|chain| {
            chain
                .into_iter()
                .rev()
                .find(|ip| !ip_is_trusted_proxy(*ip, trusted_proxies))
        })
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(parse_ip_addr)
                .filter(|ip| *ip != peer_ip)
        })
}

fn forwarded_chain_from_header(headers: &HeaderMap) -> Option<Vec<IpAddr>> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .map(parse_ip_addr)
                .collect::<Option<Vec<_>>>()
        })
        .filter(|chain| !chain.is_empty())
}

fn parse_ip_addr(value: &str) -> Option<IpAddr> {
    value.parse::<IpAddr>().ok()
}

/// 清理失败计数与封禁状态:把已过期的项目逐出,使长时间不活跃的 IP 不会被无端封禁。
pub fn prune_auth_failure_state(
    state: &mut AuthFailureState,
    now: Instant,
    failure_window: Duration,
) {
    while state
        .recent_failures
        .front()
        .is_some_and(|timestamp| now.duration_since(*timestamp) > failure_window)
    {
        state.recent_failures.pop_front();
    }

    if state
        .blocked_until
        .is_some_and(|blocked_until| blocked_until <= now)
    {
        state.blocked_until = None;
    }
}

/// 全表扫描:对每个 IP 的失败状态做一次 `prune_auth_failure_state`,然后丢弃
/// 那些既无未过期失败、也无未到期封禁的条目。代价 O(N),由 `record_auth_failure`
/// 在表大小超过软上限时触发,使被攻击场景下表的稳态体积可控。
pub fn sweep_expired_auth_failures(
    auth_failures: &mut HashMap<IpAddr, AuthFailureState>,
    now: Instant,
    failure_window: Duration,
) {
    auth_failures.retain(|_, failure_state| {
        prune_auth_failure_state(failure_state, now, failure_window);
        !failure_state.recent_failures.is_empty() || failure_state.blocked_until.is_some()
    });
}

fn trim_auth_failure_table(
    auth_failures: &mut HashMap<IpAddr, AuthFailureState>,
    now: Instant,
    failure_window: Duration,
    soft_limit: usize,
    hard_limit: usize,
) {
    if auth_failures.len() > soft_limit {
        sweep_expired_auth_failures(auth_failures, now, failure_window);
    }
    enforce_auth_failure_hard_cap(auth_failures, hard_limit);
}

fn enforce_auth_failure_hard_cap(
    auth_failures: &mut HashMap<IpAddr, AuthFailureState>,
    hard_limit: usize,
) {
    if auth_failures.len() <= hard_limit {
        return;
    }

    let remove_count = auth_failures.len() - hard_limit;
    let mut eviction_candidates: Vec<_> = auth_failures
        .iter()
        .map(|(ip, failure_state)| (*ip, auth_failure_activity_key(failure_state)))
        .collect();
    eviction_candidates.sort_by_key(|(ip, activity_key)| (*activity_key, *ip));
    for (ip, _) in eviction_candidates.into_iter().take(remove_count) {
        auth_failures.remove(&ip);
    }
}

fn auth_failure_activity_key(failure_state: &AuthFailureState) -> Option<Instant> {
    failure_state
        .blocked_until
        .or_else(|| failure_state.recent_failures.back().copied())
}

/// 把准入控制错误映射成对应的 HTTP 响应。
pub fn ws_admission_error_response(error: WsAdmissionError) -> Response {
    match error {
        WsAdmissionError::TotalCapacity => (
            StatusCode::SERVICE_UNAVAILABLE,
            "websocket capacity exhausted; retry later",
        )
            .into_response(),
        WsAdmissionError::IpCapacity => (
            StatusCode::TOO_MANY_REQUESTS,
            "too many concurrent websocket sessions for this client",
        )
            .into_response(),
        WsAdmissionError::Blocked { retry_after_secs } => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry_after_secs.to_string())],
            "too many recent websocket authentication failures",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::str::FromStr;

    use super::*;

    fn test_ws_config() -> WsConfig {
        WsConfig {
            max_total_connections: 2,
            max_connections_per_ip: 1,
            auth_fail_window_secs: 60,
            auth_fail_max_attempts: 2,
            auth_block_secs: 60,
        }
    }

    fn test_install_config() -> InstallAdmissionConfig {
        InstallAdmissionConfig {
            auth_fail_window_secs: 60,
            auth_fail_max_attempts: 2,
            auth_block_secs: 60,
        }
    }

    fn indexed_ip(index: usize) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(
            10,
            ((index >> 16) & 0xff) as u8,
            ((index >> 8) & 0xff) as u8,
            (index & 0xff) as u8,
        ))
    }

    #[test]
    fn websocket_auth_failure_table_respects_hard_cap() {
        let controller = WsAdmissionController::new(&test_ws_config());

        for index in 0..=WS_AUTH_FAILURE_TABLE_HARD_LIMIT {
            controller.record_auth_failure(indexed_ip(index));
        }

        let state = controller.state.lock().expect("lock state");
        assert_eq!(state.auth_failures.len(), WS_AUTH_FAILURE_TABLE_HARD_LIMIT);
        assert!(!state.auth_failures.contains_key(&indexed_ip(0)));
        assert!(
            state
                .auth_failures
                .contains_key(&indexed_ip(WS_AUTH_FAILURE_TABLE_HARD_LIMIT))
        );
    }

    #[test]
    fn install_auth_failure_table_respects_hard_cap() {
        let controller = InstallAdmissionController::new(test_install_config());

        for index in 0..=INSTALL_AUTH_FAILURE_TABLE_HARD_LIMIT {
            controller.record_auth_failure(indexed_ip(index));
        }

        let state = controller.state.lock().expect("lock state");
        assert_eq!(
            state.auth_failures.len(),
            INSTALL_AUTH_FAILURE_TABLE_HARD_LIMIT
        );
        assert!(!state.auth_failures.contains_key(&indexed_ip(0)));
        assert!(
            state
                .auth_failures
                .contains_key(&indexed_ip(INSTALL_AUTH_FAILURE_TABLE_HARD_LIMIT))
        );
    }

    #[test]
    fn hard_cap_preserves_single_ip_block_semantics() {
        let ws_controller = WsAdmissionController::new(&test_ws_config());
        let install_controller = InstallAdmissionController::new(test_install_config());
        let client_ip = indexed_ip(42);

        ws_controller.record_auth_failure(client_ip);
        ws_controller.record_auth_failure(client_ip);
        assert!(matches!(
            ws_controller.try_acquire(client_ip),
            Err(WsAdmissionError::Blocked { retry_after_secs }) if retry_after_secs > 0
        ));

        install_controller.record_auth_failure(client_ip);
        install_controller.record_auth_failure(client_ip);
        assert!(matches!(
            install_controller.check(client_ip),
            Err(retry_after_secs) if retry_after_secs > 0
        ));
    }

    #[test]
    fn websocket_admission_keeps_capacity_limits_after_mutex_poison() {
        let controller = WsAdmissionController::new(&test_ws_config());
        let client_ip = IpAddr::from_str("198.51.100.10").expect("valid test IP");

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut state = controller.state.lock().expect("lock state");
            state.total_active_connections = controller.config.max_total_connections;
            state
                .active_by_ip
                .insert(client_ip, controller.config.max_connections_per_ip);
            panic!("poison admission state");
        }));

        assert!(matches!(
            controller.try_acquire(client_ip),
            Err(WsAdmissionError::TotalCapacity)
        ));
        assert!(controller.state.lock().is_ok());
    }

    #[test]
    fn websocket_admission_reconciles_connection_counters_after_mutex_poison() {
        let controller = WsAdmissionController::new(&test_ws_config());
        let client_ip = IpAddr::from_str("198.51.100.12").expect("valid test IP");

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut state = controller.state.lock().expect("lock state");
            state.total_active_connections = 0;
            state.active_by_ip.insert(client_ip, 1);
            panic!("poison admission state");
        }));

        let state = controller.lock_state();
        assert_eq!(state.total_active_connections, 1);
        assert_eq!(state.active_by_ip.get(&client_ip), Some(&1));
        drop(state);
        assert!(controller.state.lock().is_ok());
    }

    #[test]
    fn install_admission_preserves_state_after_mutex_poison() {
        let controller = InstallAdmissionController::new(test_install_config());
        let client_ip = IpAddr::from_str("198.51.100.11").expect("valid test IP");

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut state = controller.state.lock().expect("lock state");
            state.auth_failures.insert(
                client_ip,
                AuthFailureState {
                    recent_failures: VecDeque::from([Instant::now(), Instant::now()]),
                    blocked_until: Some(Instant::now() + Duration::from_secs(60)),
                },
            );
            panic!("poison install admission state");
        }));

        // After poison, valid state should be preserved, not reset
        // The blocked_until is in the future, so check should return Err
        assert!(controller.check(client_ip).is_err());
        assert!(controller.state.lock().is_ok());
    }
}
