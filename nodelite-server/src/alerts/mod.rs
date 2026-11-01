//! 告警运行时:把配置规则转换为可复用的触发与巡检摘要视图。

mod channels;
mod delivery;
mod delivery_metrics;
mod evaluator;
mod runtime;
mod time_math;
mod tracker;

pub(crate) use channels::ChannelSet;
pub(crate) use delivery::{
    DeliveryOutcome, InspectionSummary, InspectionTrendPoint, deliver_alert_event,
    deliver_inspection_summary, smtp_endpoint_label, webhook_endpoint_label,
};
pub(crate) use delivery_metrics::{
    AlertDeliveryMetrics, AlertDeliverySnapshot, DELIVERY_QUEUE_CAPACITY, DELIVERY_RESULT_CAPACITY,
    DELIVERY_TOTAL_CAPACITY, MAX_CONCURRENT_DELIVERIES,
};
pub(crate) use evaluator::{
    AlertMetricReading, AlertStatusView, EvaluatedRule, InspectionHighlight,
    InspectionHighlightEvent, InspectionReport, build_inspection_report, evaluate_rules,
};
pub(crate) use runtime::spawn_alert_runtime;
pub(crate) use tracker::{AlertEvent, AlertEventKind, AlertStateTracker};
