//! 告警生命周期状态跟踪与冷却抑止模块。
//!
//! 维护已触发规则的活跃状态、抑制重复发送、管理恢复通知（Resolved）以及投递失败后的重试退避。

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use nodelite_proto::AlertRuleConfig;

use super::channels::ChannelSet;
use super::{AlertMetricReading, EvaluatedRule};

const DELIVERY_FAILURE_RETRY_DELAY_MINUTES: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlertEventKind {
    Triggered,
    Resolved,
}

impl AlertEventKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Triggered => "triggered",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertEvent {
    pub(crate) kind: AlertEventKind,
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) rule: AlertRuleConfig,
    pub(crate) node_id: String,
    pub(crate) node_label: String,
    pub(crate) reading: Option<AlertMetricReading>,
    /// Channels this event still has to reach; the rule and config filter it further.
    pub(crate) channels: ChannelSet,
}

#[derive(Debug, Default)]
pub(crate) struct AlertStateTracker {
    active: HashMap<AlertInstanceKey, ActiveAlertState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AlertInstanceKey {
    rule_id: String,
    node_id: String,
}

#[derive(Debug, Clone)]
struct ActiveAlertState {
    node_label: String,
    last_reading: AlertMetricReading,
    last_notified_at: DateTime<Utc>,
    /// A cooldown repeat starts a new round even while a failed channel is still retrying.
    round_started_at: DateTime<Utc>,
    /// Channels that received the current notification round; retries skip them.
    round_delivered: ChannelSet,
    /// False until the first delivery attempt reports back.
    outcome_seen: bool,
    /// Set while the current round still has a failed channel.
    retry_pending: bool,
    /// Channels that ever received this alert; the resolved notice goes only to them.
    ever_delivered: ChannelSet,
    resolved_at: Option<DateTime<Utc>>,
}

impl ActiveAlertState {
    /// The resolved notice waits while the trigger is in flight or failed on every channel,
    /// so no channel gets a resolution for an alert it never saw.
    fn holds_resolved_notice(&self) -> bool {
        !self.outcome_seen || (self.retry_pending && self.ever_delivered.is_empty())
    }

    /// Nothing delivered and nothing failed means no channel applied; config filtering then
    /// decides, as for any other event.
    fn resolved_channels(&self) -> ChannelSet {
        if self.ever_delivered.is_empty() {
            ChannelSet::ALL
        } else {
            self.ever_delivered
        }
    }
}

impl AlertStateTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn clear(&mut self) {
        self.active.clear();
    }

    pub(crate) fn update(
        &mut self,
        rules: &[AlertRuleConfig],
        matches: &[EvaluatedRule],
        now: DateTime<Utc>,
    ) -> Vec<AlertEvent> {
        let rules_by_id = rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| (rule.id.as_str(), rule))
            .collect::<HashMap<_, _>>();
        let mut current_keys = HashSet::new();
        let mut events = Vec::new();

        for matched in matches {
            let Some(rule) = rules_by_id.get(matched.rule_id.as_str()) else {
                continue;
            };
            let key = AlertInstanceKey {
                rule_id: matched.rule_id.clone(),
                node_id: matched.node_id.clone(),
            };
            current_keys.insert(key.clone());

            match self.active.get_mut(&key) {
                Some(active) => {
                    active.node_label.clone_from(&matched.node_label);
                    active.last_reading = matched.reading.clone();
                    active.resolved_at = None;
                    if should_repeat_alert(active.last_notified_at, rule.cooldown_minutes, now) {
                        active.last_notified_at = now;
                        if !active.retry_pending
                            || should_repeat_alert(
                                active.round_started_at,
                                rule.cooldown_minutes,
                                now,
                            )
                        {
                            active.round_started_at = now;
                            active.round_delivered = ChannelSet::NONE;
                        }
                        let mut event = triggered_event(rule, matched, now);
                        event.channels = ChannelSet::ALL.without(active.round_delivered);
                        events.push(event);
                    }
                }
                None => {
                    self.active.insert(
                        key,
                        ActiveAlertState {
                            node_label: matched.node_label.clone(),
                            last_reading: matched.reading.clone(),
                            last_notified_at: now,
                            round_started_at: now,
                            round_delivered: ChannelSet::NONE,
                            outcome_seen: false,
                            retry_pending: false,
                            ever_delivered: ChannelSet::NONE,
                            resolved_at: None,
                        },
                    );
                    events.push(triggered_event(rule, matched, now));
                }
            }
        }

        let resolved = self
            .active
            .keys()
            .filter(|key| !current_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in resolved {
            let Some(active) = self.active.remove(&key) else {
                continue;
            };
            let Some(rule) = rules_by_id.get(key.rule_id.as_str()) else {
                continue;
            };
            if active.holds_resolved_notice() {
                let mut active = active;
                active.resolved_at = Some(now);
                self.active.insert(key, active);
                continue;
            }
            if rule.send_resolved {
                let channels = active.resolved_channels();
                events.push(AlertEvent {
                    kind: AlertEventKind::Resolved,
                    occurred_at: now,
                    rule: (*rule).clone(),
                    node_id: key.node_id,
                    node_label: active.node_label,
                    reading: Some(active.last_reading),
                    channels,
                });
            }
        }

        events
    }

    /// Records which channels accepted a triggered event. A failed channel is retried after
    /// the short delay; the resolved notice is released once at least one channel has the
    /// trigger, and only goes to those channels.
    pub(crate) fn record_delivery_outcome(
        &mut self,
        event: &AlertEvent,
        delivered: ChannelSet,
        failed: bool,
        now: DateTime<Utc>,
    ) -> Option<AlertEvent> {
        if event.kind != AlertEventKind::Triggered {
            return None;
        }
        let key = AlertInstanceKey {
            rule_id: event.rule.id.clone(),
            node_id: event.node_id.clone(),
        };
        let active = self.active.get_mut(&key)?;
        active.outcome_seen = true;
        active.round_delivered = active.round_delivered.union(delivered);
        active.ever_delivered = active.ever_delivered.union(delivered);
        active.retry_pending = failed;
        if failed {
            active.last_notified_at = retry_notified_at(event.rule.cooldown_minutes, now);
        }
        let resolved_at = active.resolved_at?;
        if active.holds_resolved_notice() {
            return None;
        }
        let active = self.active.remove(&key)?;
        if !event.rule.send_resolved {
            return None;
        }
        Some(AlertEvent {
            kind: AlertEventKind::Resolved,
            occurred_at: resolved_at,
            rule: event.rule.clone(),
            node_id: event.node_id.clone(),
            channels: active.resolved_channels(),
            node_label: active.node_label,
            reading: Some(active.last_reading),
        })
    }

    #[cfg(test)]
    pub(crate) fn record_delivery_failure(&mut self, event: &AlertEvent, now: DateTime<Utc>) {
        self.record_delivery_outcome(event, ChannelSet::NONE, true, now);
    }

    #[cfg(test)]
    pub(crate) fn record_delivery_success(&mut self, event: &AlertEvent) -> Option<AlertEvent> {
        self.record_delivery_outcome(event, ChannelSet::ALL, false, Utc::now())
    }
}

fn should_repeat_alert(
    last_notified_at: DateTime<Utc>,
    cooldown_minutes: u64,
    now: DateTime<Utc>,
) -> bool {
    let cooldown_minutes = i64::try_from(cooldown_minutes).unwrap_or(i64::MAX);
    now.signed_duration_since(last_notified_at) >= Duration::minutes(cooldown_minutes)
}

fn retry_notified_at(cooldown_minutes: u64, now: DateTime<Utc>) -> DateTime<Utc> {
    let cooldown_minutes = i64::try_from(cooldown_minutes).unwrap_or(i64::MAX);
    let retry_offset = cooldown_minutes.saturating_sub(DELIVERY_FAILURE_RETRY_DELAY_MINUTES);
    now - Duration::minutes(retry_offset)
}

fn triggered_event(
    rule: &AlertRuleConfig,
    matched: &EvaluatedRule,
    now: DateTime<Utc>,
) -> AlertEvent {
    AlertEvent {
        kind: AlertEventKind::Triggered,
        occurred_at: now,
        rule: rule.clone(),
        node_id: matched.node_id.clone(),
        node_label: matched.node_label.clone(),
        reading: Some(matched.reading.clone()),
        channels: ChannelSet::ALL,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use nodelite_proto::{
        AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    };

    use super::{AlertEventKind, AlertStateTracker, ChannelSet};
    use crate::alerts::{AlertMetricReading, EvaluatedRule};

    fn rule(send_resolved: bool) -> AlertRuleConfig {
        AlertRuleConfig {
            id: "cpu-hot".to_string(),
            name: "CPU".to_string(),
            enabled: true,
            metric: AlertMetric::CpuUsagePercent,
            comparator: AlertComparator::Gt,
            threshold: 90,
            window_minutes: 5,
            severity: AlertSeverity::Critical,
            scope_mode: AlertScopeMode::All,
            node_ids: Vec::new(),
            tags: Vec::new(),
            delivery: vec![AlertChannel::Webhook],
            cooldown_minutes: 30,
            send_resolved,
        }
    }

    fn matched(value: u64) -> EvaluatedRule {
        EvaluatedRule {
            rule_id: "cpu-hot".to_string(),
            node_id: "hk-01".to_string(),
            node_label: "Hong Kong".to_string(),
            reading: AlertMetricReading {
                metric: AlertMetric::CpuUsagePercent,
                value,
                threshold: 90,
            },
        }
    }

    #[test]
    fn update_emits_trigger_once_inside_cooldown() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();

        let first = tracker.update(&rules, &[matched(91)], now);
        let second = tracker.update(&rules, &[matched(92)], now + Duration::minutes(10));

        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, AlertEventKind::Triggered);
        assert!(second.is_empty());
    }

    #[test]
    fn update_repeats_trigger_after_cooldown() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();

        let _ = tracker.update(&rules, &[matched(91)], now);
        let repeated = tracker.update(&rules, &[matched(93)], now + Duration::minutes(30));

        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].kind, AlertEventKind::Triggered);
        assert_eq!(
            repeated[0].reading.as_ref().map(|reading| reading.value),
            Some(93)
        );
    }

    #[test]
    fn record_delivery_failure_allows_early_retry() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();
        let first = tracker.update(&rules, &[matched(91)], now);

        tracker.record_delivery_failure(&first[0], now);
        let retry_too_soon = tracker.update(&rules, &[matched(92)], now + Duration::minutes(4));
        let retry = tracker.update(&rules, &[matched(93)], now + Duration::minutes(5));

        assert!(retry_too_soon.is_empty());
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].kind, AlertEventKind::Triggered);
        assert_eq!(
            retry[0].reading.as_ref().map(|reading| reading.value),
            Some(93)
        );
    }

    #[test]
    fn update_emits_resolved_when_match_disappears() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();

        let first = tracker.update(&rules, &[matched(91)], now);
        assert!(tracker.record_delivery_success(&first[0]).is_none());
        let resolved = tracker.update(&rules, &[], now + Duration::minutes(5));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].kind, AlertEventKind::Resolved);
        assert_eq!(resolved[0].node_id, "hk-01");
    }

    #[test]
    fn update_skips_resolved_event_when_rule_disables_it() {
        let now = Utc::now();
        let rules = vec![rule(false)];
        let mut tracker = AlertStateTracker::new();

        let _ = tracker.update(&rules, &[matched(91)], now);
        let resolved = tracker.update(&rules, &[], now + Duration::minutes(5));

        assert!(resolved.is_empty());
    }

    #[test]
    fn update_skips_resolved_until_trigger_delivery_succeeds() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();

        let first = tracker.update(&rules, &[matched(91)], now);
        let skipped = tracker.update(&rules, &[], now + Duration::minutes(5));

        assert_eq!(first.len(), 1);
        assert!(skipped.is_empty());
    }

    #[test]
    fn delivery_success_after_resolution_returns_pending_resolved_event() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();

        let first = tracker.update(&rules, &[matched(91)], now);
        let skipped = tracker.update(&rules, &[], now + Duration::minutes(5));
        let resolved = tracker
            .record_delivery_success(&first[0])
            .expect("resolved event should be returned once trigger delivery succeeds");

        assert!(skipped.is_empty());
        assert_eq!(resolved.kind, AlertEventKind::Resolved);
        assert_eq!(resolved.occurred_at, now + Duration::minutes(5));
        assert_eq!(resolved.node_id, "hk-01");
    }

    #[test]
    fn update_emits_resolved_after_trigger_delivery_succeeds() {
        let now = Utc::now();
        let rules = vec![rule(true)];
        let mut tracker = AlertStateTracker::new();

        let first = tracker.update(&rules, &[matched(91)], now);
        assert!(tracker.record_delivery_success(&first[0]).is_none());
        let resolved = tracker.update(&rules, &[], now + Duration::minutes(5));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].kind, AlertEventKind::Resolved);
    }

    const WEBHOOK_ONLY: ChannelSet = ChannelSet {
        webhook: true,
        smtp: false,
    };
    const SMTP_ONLY: ChannelSet = ChannelSet {
        webhook: false,
        smtp: true,
    };

    #[test]
    fn partial_failure_retries_only_the_failed_channel() {
        let rules = vec![rule(true)];
        let now = Utc::now();
        let mut tracker = AlertStateTracker::new();
        let first = tracker.update(&rules, &[matched(95)], now);
        assert_eq!(first[0].channels, ChannelSet::ALL);

        assert!(
            tracker
                .record_delivery_outcome(&first[0], WEBHOOK_ONLY, true, now)
                .is_none()
        );
        let retry = tracker.update(&rules, &[matched(95)], now + Duration::minutes(6));
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].channels, SMTP_ONLY, "webhook already has it");

        tracker.record_delivery_outcome(&retry[0], SMTP_ONLY, false, now);
        let repeat = tracker.update(&rules, &[matched(95)], now + Duration::minutes(40));
        assert_eq!(
            repeat[0].channels,
            ChannelSet::ALL,
            "a new cooldown round goes to every channel"
        );
    }

    #[test]
    fn resolved_notice_reaches_channels_that_got_the_trigger_despite_a_failing_one() {
        let rules = vec![rule(true)];
        let now = Utc::now();
        let mut tracker = AlertStateTracker::new();
        let first = tracker.update(&rules, &[matched(95)], now);
        tracker.record_delivery_outcome(&first[0], WEBHOOK_ONLY, true, now);

        let resolved = tracker.update(&rules, &[], now + Duration::minutes(1));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].kind, AlertEventKind::Resolved);
        assert_eq!(resolved[0].channels, WEBHOOK_ONLY);
        assert!(
            tracker
                .update(&rules, &[], now + Duration::minutes(10))
                .is_empty()
        );
    }

    #[test]
    fn resolved_notice_waits_until_some_channel_has_the_trigger() {
        let rules = vec![rule(true)];
        let now = Utc::now();
        let mut tracker = AlertStateTracker::new();
        let first = tracker.update(&rules, &[matched(95)], now);
        tracker.record_delivery_outcome(&first[0], ChannelSet::NONE, true, now);
        assert!(tracker.update(&rules, &[], now).is_empty());

        let resolved = tracker
            .record_delivery_outcome(&first[0], SMTP_ONLY, true, now)
            .expect("resolved notice is released once smtp has the trigger");
        assert_eq!(resolved.channels, SMTP_ONLY);
    }

    #[test]
    fn healthy_channel_still_gets_cooldown_repeats_while_another_keeps_failing() {
        let rules = vec![rule(true)];
        let now = Utc::now();
        let mut tracker = AlertStateTracker::new();
        let first = tracker.update(&rules, &[matched(95)], now);
        tracker.record_delivery_outcome(&first[0], WEBHOOK_ONLY, true, now);
        let retry_at = now + Duration::minutes(6);
        let retry = tracker.update(&rules, &[matched(95)], retry_at);
        assert_eq!(retry[0].channels, SMTP_ONLY);
        tracker.record_delivery_outcome(&retry[0], ChannelSet::NONE, true, retry_at);

        let repeat = tracker.update(&rules, &[matched(95)], now + Duration::minutes(31));
        assert_eq!(repeat.len(), 1);
        assert_eq!(
            repeat[0].channels,
            ChannelSet::ALL,
            "the cooldown boundary starts a new round for the healthy channel too"
        );
    }

    #[test]
    fn resolved_notice_is_released_when_no_channel_applied() {
        let rules = vec![rule(true)];
        let now = Utc::now();
        let mut tracker = AlertStateTracker::new();
        let first = tracker.update(&rules, &[matched(95)], now);
        assert!(
            tracker.update(&rules, &[], now).is_empty(),
            "held while the trigger is in flight"
        );

        let resolved = tracker
            .record_delivery_outcome(&first[0], ChannelSet::NONE, false, now)
            .expect("nothing failed, so the notice is not held back");
        assert_eq!(resolved.channels, ChannelSet::ALL);
    }

    #[test]
    fn update_clears_active_state_when_rule_is_disabled() {
        let now = Utc::now();
        let enabled_rule = rule(true);
        let mut disabled_rule = enabled_rule.clone();
        disabled_rule.enabled = false;
        let mut tracker = AlertStateTracker::new();

        let first = tracker.update(std::slice::from_ref(&enabled_rule), &[matched(91)], now);
        let disabled = tracker.update(&[disabled_rule], &[], now + Duration::minutes(5));
        let reenabled = tracker.update(&[enabled_rule], &[matched(92)], now + Duration::minutes(6));

        assert_eq!(first.len(), 1);
        assert!(
            disabled.is_empty(),
            "disabled rules should not emit resolved events"
        );
        assert_eq!(
            reenabled.len(),
            1,
            "re-enabled rules should trigger immediately"
        );
        assert_eq!(reenabled[0].kind, AlertEventKind::Triggered);
        assert_eq!(
            reenabled[0].reading.as_ref().map(|reading| reading.value),
            Some(92)
        );
    }
}
