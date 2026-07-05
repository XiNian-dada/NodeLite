use base64::{Engine as _, engine::general_purpose::STANDARD};
use nodelite_proto::AlertSmtpConfig;

use crate::alerts::AlertEvent;

use super::super::{AlertDeliveryError, InspectionSummary};
use chart::{TrendStats, format_metric_value, trend_chart_html, trend_has_samples, trend_stats};

mod chart;

const ALERT_BOUNDARY: &str = "nodelite-alert-alt";
const INSPECTION_BOUNDARY: &str = "nodelite-inspection-alt";

pub(super) fn build_alert_message(
    config: &AlertSmtpConfig,
    event: &AlertEvent,
) -> Result<String, AlertDeliveryError> {
    validate_header_value(&event.rule.name)?;
    validate_header_value(&event.node_label)?;
    let subject = format!(
        "[NodeLite] {} {} on {}",
        event.kind.as_str(),
        event.rule.name,
        event.node_label
    );
    validate_header_value(&subject)?;
    let recipients = config.recipients.join(", ");
    validate_header_value(&recipients)?;

    let text = alert_message_body(event);
    let html = alert_message_html(event);

    Ok(format!(
        concat!(
            "From: {}\r\n",
            "To: {}\r\n",
            "Subject: {}\r\n",
            "Date: {}\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/alternative; boundary=\"{}\"\r\n",
            "\r\n",
            "--{}\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "Content-Transfer-Encoding: 8bit\r\n",
            "\r\n",
            "{}\r\n",
            "--{}\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "Content-Transfer-Encoding: 8bit\r\n",
            "\r\n",
            "{}\r\n",
            "--{}--\r\n"
        ),
        config.sender,
        recipients,
        subject,
        event.occurred_at.to_rfc2822(),
        ALERT_BOUNDARY,
        ALERT_BOUNDARY,
        text,
        ALERT_BOUNDARY,
        html,
        ALERT_BOUNDARY,
    ))
}

pub(super) fn build_inspection_message(
    config: &AlertSmtpConfig,
    summary: &InspectionSummary<'_>,
) -> Result<String, AlertDeliveryError> {
    let subject = format!("[NodeLite] Daily inspection {}", summary.local_date);
    validate_header_value(&subject)?;
    let recipients = config.recipients.join(", ");
    validate_header_value(&recipients)?;

    let text = inspection_message_body(summary);
    let html = inspection_message_html(summary);

    Ok(format!(
        concat!(
            "From: {}\r\n",
            "To: {}\r\n",
            "Subject: {}\r\n",
            "Date: {}\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/alternative; boundary=\"{}\"\r\n",
            "\r\n",
            "--{}\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "Content-Transfer-Encoding: 8bit\r\n",
            "\r\n",
            "{}\r\n",
            "--{}\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "Content-Transfer-Encoding: 8bit\r\n",
            "\r\n",
            "{}\r\n",
            "--{}--\r\n"
        ),
        config.sender,
        recipients,
        subject,
        summary.occurred_at.to_rfc2822(),
        INSPECTION_BOUNDARY,
        INSPECTION_BOUNDARY,
        text,
        INSPECTION_BOUNDARY,
        html,
        INSPECTION_BOUNDARY,
    ))
}

fn alert_message_body(event: &AlertEvent) -> String {
    let mut body = format!(
        "NodeLite alert {}\n\nRule: {} ({})\nSeverity: {:?}\nNode: {} ({})\nTime: {}\nWindow: {}\n",
        event.kind.as_str(),
        event.rule.name,
        event.rule.id,
        event.rule.severity,
        event.node_label,
        event.node_id,
        event.occurred_at.to_rfc3339(),
        alert_window_label(event),
    );
    if let Some(reading) = event.reading.as_ref() {
        body.push_str(&format!(
            "Metric: {}\nValue: {}\nThreshold: {} {}\n",
            alert_metric_label(&reading.metric),
            format_alert_metric_value(&reading.metric, reading.value),
            alert_comparator_symbol(&event.rule.comparator),
            format_alert_metric_value(&reading.metric, reading.threshold),
        ));
    }
    body
}

fn alert_message_html(event: &AlertEvent) -> String {
    let logo_src = brand_logo_data_uri();
    let status = alert_status_label(event);
    let status_style = alert_status_style(event);
    let reading = alert_reading_rows_html(event);

    format!(
        concat!(
            r#"<!doctype html><html><head><meta charset="utf-8"></head><body style="margin:0;padding:0;background:#f5f5f5;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC','Helvetica Neue',Arial,sans-serif;color:#111111;">"#,
            r#"<div style="max-width:680px;margin:0 auto;padding:24px 16px;">"#,
            r#"<div style="overflow:hidden;border:1px solid #e5e5e5;border-radius:14px;background:#ffffff;">"#,
            r#"<div style="padding:22px 26px;border-bottom:1px solid #eeeeee;">"#,
            r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse;"><tr>"#,
            r#"<td style="vertical-align:top;width:60px;padding-right:14px;"><img src="{}" width="48" height="48" alt="NodeLite" style="display:block;width:48px;height:48px;border:0;border-radius:10px;"/></td>"#,
            r#"<td style="vertical-align:top;"><div style="font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:#6b6b6b;">NodeLite Alert Notification</div>"#,
            r#"<h1 style="margin:6px 0 0;font-size:22px;line-height:1.25;font-weight:700;color:#111111;">{}</h1>"#,
            r#"<p style="margin:8px 0 0;color:#6b6b6b;font-size:13px;">{} &#183; {}</p></td>"#,
            r#"<td style="vertical-align:top;text-align:right;"><span style="display:inline-block;border:1px solid {};border-radius:999px;background:{};color:{};padding:5px 10px;font-size:12px;font-weight:650;">{}</span></td>"#,
            r#"</tr></table></div>"#,
            r#"<div style="padding:20px 26px 24px;">"#,
            r#"<div style="border:1px solid #e5e5e5;border-radius:12px;background:#ffffff;padding:16px 18px;">"#,
            r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse;">"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;width:132px;">Node</td><td style="padding:6px 0;color:#111111;font-size:14px;font-weight:650;">{}</td></tr>"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Node ID</td><td style="padding:6px 0;color:#111111;font-size:14px;">{}</td></tr>"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Rule ID</td><td style="padding:6px 0;color:#111111;font-size:14px;">{}</td></tr>"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Severity</td><td style="padding:6px 0;color:#111111;font-size:14px;">{}</td></tr>"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Window</td><td style="padding:6px 0;color:#111111;font-size:14px;">{}</td></tr>"#,
            "{}",
            r#"</table></div>"#,
            r#"<p style="margin:16px 0 0;color:#8b8b93;font-size:12px;">Daily inspection reports are sent separately from realtime alert notifications.</p>"#,
            r#"</div></div></div></body></html>"#
        ),
        logo_src,
        escape_html(&event.rule.name),
        escape_html(&format_event_time(event.occurred_at)),
        escape_html(&event.occurred_at.to_rfc3339()),
        status_style.border,
        status_style.background,
        status_style.color,
        status,
        escape_html(&event.node_label),
        escape_html(&event.node_id),
        escape_html(&event.rule.id),
        escape_html(&format!("{:?}", event.rule.severity)),
        escape_html(&alert_window_label(event)),
        reading,
    )
}

struct AlertStatusStyle {
    border: &'static str,
    background: &'static str,
    color: &'static str,
}

fn alert_status_label(event: &AlertEvent) -> &'static str {
    match event.kind {
        crate::alerts::AlertEventKind::Resolved => "Resolved",
        crate::alerts::AlertEventKind::Triggered => match event.rule.severity {
            nodelite_proto::AlertSeverity::Critical => "Critical",
            nodelite_proto::AlertSeverity::Warning => "Warning",
        },
    }
}

fn alert_status_style(event: &AlertEvent) -> AlertStatusStyle {
    match event.kind {
        crate::alerts::AlertEventKind::Resolved => AlertStatusStyle {
            border: "#bbf7d0",
            background: "#f0fdf4",
            color: "#15803d",
        },
        crate::alerts::AlertEventKind::Triggered => match event.rule.severity {
            nodelite_proto::AlertSeverity::Critical => AlertStatusStyle {
                border: "#fecdd3",
                background: "#fff1f2",
                color: "#be123c",
            },
            nodelite_proto::AlertSeverity::Warning => AlertStatusStyle {
                border: "#fde68a",
                background: "#fffbeb",
                color: "#a16207",
            },
        },
    }
}

fn alert_reading_rows_html(event: &AlertEvent) -> String {
    let Some(reading) = event.reading.as_ref() else {
        return String::new();
    };

    format!(
        concat!(
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Metric</td><td style="padding:6px 0;color:#111111;font-size:14px;">{}</td></tr>"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Value</td><td style="padding:6px 0;color:#111111;font-size:14px;font-weight:650;">{}</td></tr>"#,
            r#"<tr><td style="padding:6px 14px 6px 0;color:#6b6b6b;font-size:12px;">Threshold</td><td style="padding:6px 0;color:#111111;font-size:14px;">{} {}</td></tr>"#
        ),
        escape_html(alert_metric_label(&reading.metric)),
        escape_html(&format_alert_metric_value(&reading.metric, reading.value)),
        escape_html(alert_comparator_symbol(&event.rule.comparator)),
        escape_html(&format_alert_metric_value(
            &reading.metric,
            reading.threshold
        )),
    )
}

fn alert_window_label(event: &AlertEvent) -> String {
    if matches!(
        event.rule.metric,
        nodelite_proto::AlertMetric::OfflineMinutes
    ) {
        return "current offline duration".to_string();
    }
    if event.rule.window_minutes <= 1 {
        return "latest sample".to_string();
    }
    format!("{} min average", event.rule.window_minutes)
}

fn alert_metric_label(metric: &nodelite_proto::AlertMetric) -> &'static str {
    match metric {
        nodelite_proto::AlertMetric::CpuUsagePercent => "CPU usage",
        nodelite_proto::AlertMetric::MemoryUsagePercent => "Memory usage",
        nodelite_proto::AlertMetric::DiskUsagePercent => "Disk usage",
        nodelite_proto::AlertMetric::LatencyMs => "RTT",
        nodelite_proto::AlertMetric::OfflineMinutes => "Offline duration",
    }
}

fn format_alert_metric_value(metric: &nodelite_proto::AlertMetric, value: u64) -> String {
    match metric {
        nodelite_proto::AlertMetric::CpuUsagePercent
        | nodelite_proto::AlertMetric::MemoryUsagePercent
        | nodelite_proto::AlertMetric::DiskUsagePercent => format!("{value}%"),
        nodelite_proto::AlertMetric::LatencyMs => format!("{value} ms"),
        nodelite_proto::AlertMetric::OfflineMinutes => format!("{value} min"),
    }
}

fn alert_comparator_symbol(comparator: &nodelite_proto::AlertComparator) -> &'static str {
    match comparator {
        nodelite_proto::AlertComparator::Gt => ">",
        nodelite_proto::AlertComparator::Lt => "<",
    }
}

fn inspection_message_body(summary: &InspectionSummary<'_>) -> String {
    let report = summary.report;
    let mut body = format!(
        "NodeLite daily inspection summary\n\nDate: {}\nLookback: {}h\nGenerated: {}\n\nTotal nodes: {}\nOffline: {}\nHigh latency: {}\nCPU hot: {}\nMemory hot: {}\n",
        summary.local_date,
        summary.lookback_hours,
        summary.occurred_at.to_rfc3339(),
        report.total_nodes,
        report.offline_nodes,
        report.latency_nodes,
        report.cpu_hot_nodes,
        report.memory_hot_nodes,
    );
    body.push_str(&inspection_trends_body(summary));
    if !report.highlights.is_empty() {
        body.push_str("\nHighlights:\n");
        for highlight in report.highlights.iter().take(20) {
            for event in &highlight.events {
                body.push_str(&format!(
                    "- {} / {} ({}): {}\n",
                    format_event_time(event.occurred_at),
                    highlight.node_label,
                    highlight.node_id,
                    event.summary
                ));
            }
        }
        if report.highlights.len() > 20 {
            body.push_str(&format!(
                "- ... {} more nodes\n",
                report.highlights.len() - 20
            ));
        }
    }
    body
}

fn inspection_message_html(summary: &InspectionSummary<'_>) -> String {
    let report = summary.report;
    let logo_src = brand_logo_data_uri();
    let cards = inspection_summary_cards_html(report, summary.lookback_hours);
    let trends = inspection_trends_html(summary);
    let highlights = inspection_highlights_html(report);

    format!(
        concat!(
            r#"<!doctype html><html><head><meta charset="utf-8"></head><body style="margin:0;padding:0;background:#f5f5f5;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC','Helvetica Neue',Arial,sans-serif;color:#111111;">"#,
            r#"<div style="max-width:760px;margin:0 auto;padding:24px 16px;">"#,
            r#"<div style="overflow:hidden;border:1px solid #e5e5e5;border-radius:14px;background:#ffffff;">"#,
            r#"<div style="padding:24px 28px;border-bottom:1px solid #eeeeee;">"#,
            r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse;"><tr>"#,
            r#"<td style="vertical-align:top;width:76px;padding-right:14px;"><img src="{}" width="64" height="64" alt="NodeLite" style="display:block;width:64px;height:64px;border:0;border-radius:12px;"/></td>"#,
            r#"<td style="vertical-align:top;"><div style="font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:#6b6b6b;">NodeLite Daily Inspection</div>"#,
            r#"<h1 style="margin:6px 0 0;font-size:24px;line-height:1.25;font-weight:700;color:#111111;">{}</h1>"#,
            r#"<p style="margin:8px 0 0;color:#6b6b6b;font-size:13px;">Generated {} &#183; lookback {}h</p></td>"#,
            r#"<td style="vertical-align:top;text-align:right;"><span style="display:inline-block;border:1px solid #dbeafe;border-radius:999px;background:#eff6ff;color:#2563eb;padding:5px 10px;font-size:12px;font-weight:600;">{} nodes</span></td>"#,
            r#"</tr></table></div>"#,
            r#"<div style="padding:20px 28px 24px;">"#,
            r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse;table-layout:fixed;">{}</table>"#,
            "{}",
            r#"<div style="margin-top:18px;border:1px solid #e5e5e5;border-radius:12px;background:#ffffff;padding:18px 20px;">"#,
            r#"<h2 style="margin:0 0 12px;font-size:16px;line-height:1.3;color:#111111;">Inspection highlights</h2>"#,
            "{}",
            r#"</div><p style="margin:18px 0 0;color:#8b8b93;font-size:12px;">This report was generated by NodeLite. Trends are aggregated from retained history samples across reporting nodes.</p>"#,
            r#"</div></div></div></body></html>"#
        ),
        logo_src,
        escape_html(&summary.local_date.to_string()),
        escape_html(&summary.occurred_at.to_rfc3339()),
        summary.lookback_hours,
        report.total_nodes,
        cards,
        trends,
        highlights,
    )
}

fn brand_logo_data_uri() -> String {
    let logo = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/public/assets/brand-logo-light.png"
    ));
    format!("data:image/png;base64,{}", STANDARD.encode(logo))
}

fn inspection_summary_cards_html(
    report: &crate::alerts::InspectionReport,
    lookback_hours: u64,
) -> String {
    let cards = [
        (
            "Total nodes",
            report.total_nodes.to_string(),
            "#2563eb",
            "#eff6ff",
        ),
        (
            "Offline",
            report.offline_nodes.to_string(),
            "#ff4d6d",
            "#fff1f2",
        ),
        (
            "High latency",
            report.latency_nodes.to_string(),
            "#d29922",
            "#fffbeb",
        ),
        (
            "CPU hot",
            report.cpu_hot_nodes.to_string(),
            "#f97316",
            "#fff7ed",
        ),
        (
            "Memory hot",
            report.memory_hot_nodes.to_string(),
            "#3b82f6",
            "#eff6ff",
        ),
        (
            "Lookback",
            format!("{lookback_hours}h"),
            "#6b7280",
            "#f8fafc",
        ),
    ];

    let mut html = String::new();
    for row in cards.chunks(3) {
        html.push_str("<tr>");
        for (label, value, color, background) in row {
            html.push_str(&format!(
                r#"<td style="padding:0 8px 8px 0;width:33.333%;vertical-align:top;"><div style="height:88px;box-sizing:border-box;border:1px solid #e5e5e5;border-radius:10px;padding:13px 14px;background:{};"><div style="height:32px;font-size:12px;line-height:16px;color:#6b6b6b;">{}</div><div style="margin-top:7px;font-size:26px;line-height:26px;font-weight:750;color:{};">{}</div></div></td>"#,
                background,
                escape_html(label),
                color,
                value,
            ));
        }
        html.push_str("</tr>");
    }
    html
}

fn inspection_trends_body(summary: &InspectionSummary<'_>) -> String {
    if !trend_has_samples(summary.trends) {
        return "\n24h trends: no retained history samples available.\n".to_string();
    }

    let mut body = String::from("\n24h trends:\n");
    push_trend_body_line(
        &mut body,
        "CPU",
        "%",
        trend_stats(summary.trends, |point| point.cpu_usage_percent),
    );
    push_trend_body_line(
        &mut body,
        "Memory",
        "%",
        trend_stats(summary.trends, |point| point.memory_used_percent),
    );
    push_trend_body_line(
        &mut body,
        "Latency",
        "ms",
        trend_stats(summary.trends, |point| point.latency_ms),
    );
    body
}

fn push_trend_body_line(body: &mut String, label: &str, unit: &str, stats: Option<TrendStats>) {
    if let Some(stats) = stats {
        body.push_str(&format!(
            "- {label}: latest {}, avg {}, peak {}\n",
            format_metric_value(stats.latest, unit),
            format_metric_value(stats.average, unit),
            format_metric_value(stats.peak, unit),
        ));
    }
}

fn inspection_trends_html(summary: &InspectionSummary<'_>) -> String {
    if !trend_has_samples(summary.trends) {
        return String::from(concat!(
            r#"<div style="margin-top:18px;border:1px dashed #d4d4d8;border-radius:12px;background:#fafafa;padding:18px 20px;">"#,
            r#"<h2 style="margin:0 0 6px;font-size:16px;line-height:1.3;color:#111111;">24h trends</h2>"#,
            r#"<p style="margin:0;color:#6b6b6b;font-size:13px;">No retained history samples were available for this inspection window.</p>"#,
            r#"</div>"#
        ));
    }

    format!(
        concat!(
            r#"<div style="margin-top:18px;border:1px solid #e5e5e5;border-radius:12px;background:#ffffff;padding:18px 20px;">"#,
            r#"<h2 style="margin:0;font-size:16px;line-height:1.3;color:#111111;">24h trends</h2>"#,
            r#"<p style="margin:5px 0 14px;color:#6b6b6b;font-size:13px;">Hourly averages across nodes with retained history samples.</p>"#,
            "{}{}{}",
            r#"</div>"#
        ),
        trend_chart_html(
            "CPU usage",
            "%",
            "#22c55e",
            "#dcfce7",
            "cpu",
            Some(100),
            summary.trends,
            |point| point.cpu_usage_percent,
        ),
        trend_chart_html(
            "Memory usage",
            "%",
            "#3b82f6",
            "#dbeafe",
            "memory",
            Some(100),
            summary.trends,
            |point| point.memory_used_percent,
        ),
        trend_chart_html(
            "Latency",
            "ms",
            "#eab308",
            "#fef3c7",
            "latency",
            None,
            summary.trends,
            |point| { point.latency_ms },
        ),
    )
}

fn format_event_time(occurred_at: chrono::DateTime<chrono::Utc>) -> String {
    occurred_at.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn inspection_highlights_html(report: &crate::alerts::InspectionReport) -> String {
    if report.highlights.is_empty() {
        return r#"<p style="margin:0;color:#16a34a;font-size:14px;">No notable nodes in this inspection window.</p>"#
            .to_string();
    }

    let mut html = String::from(
        r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse;">"#,
    );
    for highlight in report.highlights.iter().take(20) {
        let events = highlight
            .events
            .iter()
            .map(|event| {
                format!(
                    concat!(
                        r#"<tr><td style="width:142px;padding:6px 10px 6px 0;color:#8b8b93;font-size:12px;vertical-align:top;white-space:nowrap;">{}</td>"#,
                        r#"<td style="padding:6px 0;color:#111111;font-size:13px;vertical-align:top;">{}</td></tr>"#
                    ),
                    escape_html(&format_event_time(event.occurred_at)),
                    escape_html(&event.summary)
                )
            })
            .collect::<String>();
        html.push_str(&format!(
            concat!(
                r#"<tr><td style="padding:10px 0;border-top:1px solid #eeeeee;">"#,
                r#"<div style="font-size:14px;font-weight:650;color:#111111;">{}</div>"#,
                r#"<div style="margin-top:2px;font-size:12px;color:#6b6b6b;">{}</div>"#,
                r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="margin-top:8px;border-collapse:collapse;">{}</table>"#,
                r#"</td></tr>"#
            ),
            escape_html(&highlight.node_label),
            escape_html(&highlight.node_id),
            events,
        ));
    }
    if report.highlights.len() > 20 {
        html.push_str(&format!(
            r#"<tr><td colspan="2" style="padding:10px 0 0;color:#6b6b6b;font-size:13px;">{} more nodes omitted.</td></tr>"#,
            report.highlights.len() - 20
        ));
    }
    html.push_str("</table>");
    html
}

pub(super) fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub(super) fn validate_header_value(value: &str) -> Result<(), AlertDeliveryError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(AlertDeliveryError::InvalidMailHeader);
    }
    Ok(())
}
