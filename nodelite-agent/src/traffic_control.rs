//! Agent 侧的套餐流量限速执行器。
//!
//! Linux 上仅创建一个 `clsact` qdisc，并使用固定优先级的 ingress/egress
//! police filter；绝不替换 root qdisc，也不会删除不属于 NodeLite 的规则。这样
//! 即使限速配置被撤销，主机原有的整形策略仍保持不变。

use std::time::Duration;

use thiserror::Error;
#[cfg(target_os = "linux")]
use tokio::process::Command;
use tokio::time::Instant;

use nodelite_proto::{TrafficControlState, TrafficControlStatus, TrafficControlUnavailableReason};

mod capability;
#[cfg(all(test, target_os = "linux"))]
mod linux_system_tests;
#[cfg(test)]
mod retry_tests;

#[cfg(any(target_os = "linux", test))]
const FILTER_PRIORITY: &str = "49152";
#[cfg(any(target_os = "linux", test))]
const FILTER_HANDLE: &str = "0x4e4c";
const MAX_TRAFFIC_RATE_KBPS: u64 = 100_000_000;
const MAX_APPLY_ATTEMPTS: u32 = 10;

/// 网络限速操作的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrafficControlOutcome {
    /// Linux 规则已经应用或撤销。
    Applied,
    /// 当前平台没有可安全使用的实现。
    Unavailable,
}

/// Agent 只跟踪自身已确认过的配置，避免重复执行 `tc`。
#[derive(Default)]
pub(crate) struct TrafficController {
    last_applied_rate_kbps: Option<Option<u64>>,
    status: Option<TrafficControlStatus>,
    desired_rate_kbps: Option<Option<u64>>,
    retry_at: Option<Instant>,
    attempts: u32,
}

/// 应用限速时发生的可恢复错误。
#[derive(Debug, Error)]
pub(crate) enum TrafficControlError {
    #[error("network traffic rate must be between 1 and {MAX_TRAFFIC_RATE_KBPS} kbit/s")]
    InvalidRate,
    #[cfg(target_os = "linux")]
    #[error("could not find a valid default-route network interface")]
    DefaultRouteNotFound,
    #[cfg(target_os = "linux")]
    #[error("failed to run tc while {operation}: {source}")]
    Command {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[cfg(target_os = "linux")]
    #[error("tc failed while {operation} (exit code {status_code:?}): {stderr}")]
    CommandFailed {
        operation: &'static str,
        status_code: Option<i32>,
        stderr: String,
    },
    #[cfg(target_os = "linux")]
    #[error("tc timed out while {operation}")]
    CommandTimeout { operation: &'static str },
    #[cfg(test)]
    #[error("injected tc failure")]
    Injected,
}

impl TrafficController {
    pub(crate) fn probe(&mut self) {
        let reason = capability::unavailable_reason();
        self.status = Some(TrafficControlStatus {
            state: if reason.is_some() {
                TrafficControlState::Unavailable
            } else {
                TrafficControlState::Ready
            },
            reason,
            desired_rate_kbps: None,
            applied_rate_kbps: None,
        });
    }

    pub(crate) fn status(&self) -> Option<TrafficControlStatus> {
        self.status.clone()
    }

    pub(crate) fn retry_policy(&self) -> Option<(Instant, Option<u64>)> {
        self.retry_at.zip(self.desired_rate_kbps)
    }

    pub(crate) async fn apply(
        &mut self,
        rate_kbps: Option<u64>,
    ) -> Result<TrafficControlOutcome, TrafficControlError> {
        self.apply_with(rate_kbps, capability::unavailable_reason(), || async {
            #[cfg(target_os = "linux")]
            {
                apply_linux_traffic_rate(rate_kbps).await
            }
            #[cfg(not(target_os = "linux"))]
            {
                Ok(())
            }
        })
        .await
    }

    async fn apply_with<F, Fut>(
        &mut self,
        rate_kbps: Option<u64>,
        unavailable: Option<TrafficControlUnavailableReason>,
        operation: F,
    ) -> Result<TrafficControlOutcome, TrafficControlError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), TrafficControlError>>,
    {
        if rate_kbps.is_some_and(|rate| rate == 0 || rate > MAX_TRAFFIC_RATE_KBPS) {
            return Err(TrafficControlError::InvalidRate);
        }
        if self.desired_rate_kbps != Some(rate_kbps) {
            self.desired_rate_kbps = Some(rate_kbps);
            self.attempts = 0;
        }
        self.retry_at = None;
        self.status = Some(TrafficControlStatus {
            state: TrafficControlState::Unavailable,
            reason: unavailable,
            desired_rate_kbps: rate_kbps,
            applied_rate_kbps: None,
        });
        if let Some(reason) = unavailable {
            if reason == TrafficControlUnavailableReason::MissingTc {
                self.schedule_retry();
            }
            return Ok(TrafficControlOutcome::Unavailable);
        }
        if self.last_applied_rate_kbps == Some(rate_kbps) {
            self.record_applied(rate_kbps);
            return Ok(TrafficControlOutcome::Applied);
        }
        if let Err(error) = operation().await {
            // A partially changed kernel policy no longer matches the last confirmed value.
            self.last_applied_rate_kbps = None;
            self.schedule_retry();
            if let Some(status) = &mut self.status {
                status.state = if self.retry_at.is_some() {
                    TrafficControlState::Retrying
                } else {
                    TrafficControlState::Failed
                };
            }
            return Err(error);
        }
        self.last_applied_rate_kbps = Some(rate_kbps);
        self.attempts = 0;
        self.record_applied(rate_kbps);
        Ok(TrafficControlOutcome::Applied)
    }

    fn schedule_retry(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts < MAX_APPLY_ATTEMPTS {
            let delay = Duration::from_secs((1_u64 << self.attempts.min(6)).min(60));
            self.retry_at = Some(Instant::now() + delay);
        }
    }

    fn record_applied(&mut self, rate_kbps: Option<u64>) {
        if let Some(status) = &mut self.status {
            status.state = TrafficControlState::Applied;
            status.applied_rate_kbps = rate_kbps;
        }
    }
}

#[cfg(target_os = "linux")]
async fn apply_linux_traffic_rate(rate_kbps: Option<u64>) -> Result<(), TrafficControlError> {
    let interface = default_route_interface()?;
    match rate_kbps {
        Some(rate_kbps) => {
            ensure_clsact(&interface).await?;
            replace_police_filter(&interface, "ingress", rate_kbps).await?;
            replace_police_filter(&interface, "egress", rate_kbps).await?;
        }
        None => {
            delete_police_filter_if_present(&interface, "ingress").await?;
            delete_police_filter_if_present(&interface, "egress").await?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn default_route_interface() -> Result<String, TrafficControlError> {
    let routes = std::fs::read_to_string("/proc/net/route").map_err(|source| {
        TrafficControlError::Command {
            operation: "reading /proc/net/route",
            source,
        }
    })?;
    parse_default_route_interface(&routes).ok_or(TrafficControlError::DefaultRouteNotFound)
}

#[cfg(target_os = "linux")]
async fn ensure_clsact(interface: &str) -> Result<(), TrafficControlError> {
    let existing = tc_output("reading qdisc configuration", qdisc_show_args(interface)).await?;
    if existing.contains("qdisc clsact") {
        return Ok(());
    }
    tc_success("adding clsact qdisc", qdisc_add_args(interface)).await
}

#[cfg(target_os = "linux")]
async fn replace_police_filter(
    interface: &str,
    direction: &str,
    rate_kbps: u64,
) -> Result<(), TrafficControlError> {
    tc_success(
        "applying traffic police filter",
        police_filter_args(interface, direction, rate_kbps),
    )
    .await
}

#[cfg(target_os = "linux")]
async fn delete_police_filter_if_present(
    interface: &str,
    direction: &str,
) -> Result<(), TrafficControlError> {
    let existing = tc_output(
        "reading traffic filters",
        filter_show_args(interface, direction),
    )
    .await?;
    if !has_nodelite_police_filter(&existing) {
        return Ok(());
    }
    tc_success(
        "removing traffic police filter",
        filter_delete_args(interface, direction),
    )
    .await
}

#[cfg(target_os = "linux")]
async fn tc_output(
    operation: &'static str,
    arguments: Vec<String>,
) -> Result<String, TrafficControlError> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("tc")
            .args(arguments)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| TrafficControlError::CommandTimeout { operation })?
    .map_err(|source| TrafficControlError::Command { operation, source })?;
    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        nodelite_proto::truncate_string_to_byte_boundary(&mut stderr, 512);
        return Err(TrafficControlError::CommandFailed {
            operation,
            status_code: output.status.code(),
            stderr,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
async fn tc_success(
    operation: &'static str,
    arguments: Vec<String>,
) -> Result<(), TrafficControlError> {
    tc_output(operation, arguments).await.map(|_| ())
}

#[cfg(target_os = "linux")]
fn qdisc_show_args(interface: &str) -> Vec<String> {
    vec![
        "qdisc".to_string(),
        "show".to_string(),
        "dev".to_string(),
        interface.to_string(),
    ]
}

#[cfg(target_os = "linux")]
fn qdisc_add_args(interface: &str) -> Vec<String> {
    vec![
        "qdisc".to_string(),
        "add".to_string(),
        "dev".to_string(),
        interface.to_string(),
        "clsact".to_string(),
    ]
}

#[cfg(target_os = "linux")]
fn filter_show_args(interface: &str, direction: &str) -> Vec<String> {
    vec![
        "filter".to_string(),
        "show".to_string(),
        "dev".to_string(),
        interface.to_string(),
        direction.to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
fn filter_delete_args(interface: &str, direction: &str) -> Vec<String> {
    vec![
        "filter".to_string(),
        "del".to_string(),
        "dev".to_string(),
        interface.to_string(),
        direction.to_string(),
        "pref".to_string(),
        FILTER_PRIORITY.to_string(),
        "handle".to_string(),
        FILTER_HANDLE.to_string(),
        "protocol".to_string(),
        "all".to_string(),
        "matchall".to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
fn police_filter_args(interface: &str, direction: &str, rate_kbps: u64) -> Vec<String> {
    let burst_kbit = (rate_kbps / 10).clamp(16, 4_096);
    vec![
        "filter".to_string(),
        "replace".to_string(),
        "dev".to_string(),
        interface.to_string(),
        direction.to_string(),
        "pref".to_string(),
        FILTER_PRIORITY.to_string(),
        "handle".to_string(),
        FILTER_HANDLE.to_string(),
        "protocol".to_string(),
        "all".to_string(),
        "matchall".to_string(),
        "action".to_string(),
        "police".to_string(),
        "rate".to_string(),
        format!("{rate_kbps}kbit"),
        "burst".to_string(),
        format!("{burst_kbit}kbit"),
        "conform-exceed".to_string(),
        "drop/ok".to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
fn has_nodelite_police_filter(output: &str) -> bool {
    let priority = format!("pref {FILTER_PRIORITY}");
    let handle = format!("handle {FILTER_HANDLE}");
    output
        .lines()
        .any(|line| line.contains(&priority) && line.contains("matchall") && line.contains(&handle))
}

#[cfg(any(target_os = "linux", test))]
fn parse_default_route_interface(routes: &str) -> Option<String> {
    routes.lines().skip(1).find_map(|line| {
        let mut fields = line.split_ascii_whitespace();
        let interface = fields.next()?;
        let destination = fields.next()?;
        (destination == "00000000" && valid_interface_name(interface))
            .then(|| interface.to_string())
    })
}

#[cfg(any(target_os = "linux", test))]
fn valid_interface_name(interface: &str) -> bool {
    !interface.is_empty()
        && interface.len() <= 15
        && interface
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::{has_nodelite_police_filter, parse_default_route_interface, valid_interface_name};

    #[test]
    fn parses_a_valid_default_route_interface() {
        let routes = "Iface\tDestination\tGateway\tFlags\nlo\t0000007F\t00000000\t0001\neth0\t00000000\t0102A8C0\t0003\n";

        assert_eq!(
            parse_default_route_interface(routes).as_deref(),
            Some("eth0")
        );
    }

    #[test]
    fn ignores_invalid_default_route_interface_names() {
        let routes = "Iface Destination Gateway Flags\nbad;name 00000000 00000000 0003\n";

        assert_eq!(parse_default_route_interface(routes), None);
        assert!(valid_interface_name("enp0s3.100"));
        assert!(!valid_interface_name("bad name"));
    }

    #[test]
    fn identifies_only_the_nodelite_traffic_filter() {
        assert!(has_nodelite_police_filter(
            "filter protocol all pref 49152 matchall chain 0 handle 0x4e4c\n"
        ));
        assert!(!has_nodelite_police_filter(
            "filter protocol all pref 49152 matchall chain 0 handle 0x1\n"
        ));
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn unsupported_platforms_do_not_attempt_system_configuration() {
        let mut controller = super::TrafficController::default();

        assert!(matches!(
            controller.apply(Some(10_000)).await,
            Ok(super::TrafficControlOutcome::Unavailable)
        ));
    }

    #[tokio::test]
    async fn rejects_rates_outside_the_server_safe_range() {
        let mut controller = super::TrafficController::default();

        assert!(matches!(
            controller.apply(Some(0)).await,
            Err(super::TrafficControlError::InvalidRate)
        ));
        assert!(matches!(
            controller
                .apply(Some(super::MAX_TRAFFIC_RATE_KBPS + 1))
                .await,
            Err(super::TrafficControlError::InvalidRate)
        ));
    }

    #[test]
    fn police_filter_uses_the_fixed_priority_and_requested_rate() {
        let args = super::police_filter_args("eth0", "egress", 10_000);

        assert!(args.windows(2).any(|pair| pair == ["pref", "49152"]));
        assert!(args.windows(2).any(|pair| pair == ["handle", "0x4e4c"]));
        assert!(args.windows(2).any(|pair| pair == ["rate", "10000kbit"]));
        assert!(args.windows(2).any(|pair| pair == ["burst", "1000kbit"]));
    }

    #[test]
    fn delete_filter_targets_only_the_nodelite_handle() {
        let args = super::filter_delete_args("eth0", "ingress");

        assert!(args.windows(2).any(|pair| pair == ["pref", "49152"]));
        assert!(args.windows(2).any(|pair| pair == ["handle", "0x4e4c"]));
        assert!(args.windows(2).any(|pair| pair == ["protocol", "all"]));
        assert!(args.iter().any(|arg| arg == "matchall"));
    }
}
