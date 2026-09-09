//! Alert evaluation uses the same registry snapshot as the live node views.

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
