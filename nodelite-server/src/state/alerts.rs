//! 告警评估注册表视图同步模块：使用与在线节点一致的内存快照。

use chrono::{DateTime, Utc};

use nodelite_proto::{AlertRuleConfig, InspectionConfig};

use crate::alerts::{EvaluatedRule, InspectionReport};

use super::SharedState;

impl SharedState {
    pub(crate) async fn evaluate_alert_rules(
        &self,
        rules: &[AlertRuleConfig],
        now: DateTime<Utc>,
    ) -> Vec<EvaluatedRule> {
        self.registry.evaluate_alert_rules(rules, now)
    }

    pub(crate) async fn build_alert_inspection_report(
        &self,
        inspection: &InspectionConfig,
        now: DateTime<Utc>,
    ) -> InspectionReport {
        self.registry.build_alert_inspection_report(inspection, now)
    }
}
