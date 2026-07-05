use super::super::super::InspectionTrendPoint;
use super::escape_html;

pub(super) fn trend_chart_html<F>(
    label: &str,
    unit: &str,
    color: &str,
    fill: &str,
    id_suffix: &str,
    fixed_scale_max: Option<u64>,
    trends: &[InspectionTrendPoint],
    selector: F,
) -> String
where
    F: Fn(&InspectionTrendPoint) -> Option<u64> + Copy,
{
    let stats = trend_stats(trends, selector);
    let scale_max = trend_scale_max(stats, fixed_scale_max);
    let stats_label = stats
        .map(|stats| {
            format!(
                "Latest {} / Avg {} / Peak {}",
                format_metric_value(stats.latest, unit),
                format_metric_value(stats.average, unit),
                format_metric_value(stats.peak, unit),
            )
        })
        .unwrap_or_else(|| "No samples".to_string());
    let chart = trend_svg_html(trends, color, fill, id_suffix, scale_max, unit, selector);

    format!(
        concat!(
            r#"<div style="margin-top:14px;padding-top:14px;border-top:1px solid #eeeeee;">"#,
            r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse;"><tr>"#,
            r#"<td style="font-size:13px;font-weight:700;color:#111111;">{}</td>"#,
            r#"<td style="text-align:right;font-size:12px;color:#6b6b6b;">{}</td>"#,
            r#"</tr></table>"#,
            r#"<div style="margin-top:8px;border:1px solid #eeeeee;border-radius:8px;background:#ffffff;overflow:hidden;">{}</div>"#,
            r#"</div>"#
        ),
        escape_html(label),
        escape_html(&stats_label),
        chart,
    )
}

fn trend_svg_html<F>(
    trends: &[InspectionTrendPoint],
    color: &str,
    fill: &str,
    id_suffix: &str,
    scale_max: u64,
    unit: &str,
    selector: F,
) -> String
where
    F: Fn(&InspectionTrendPoint) -> Option<u64>,
{
    const WIDTH: f64 = 640.0;
    const HEIGHT: f64 = 196.0;
    const PAD_LEFT: f64 = 76.0;
    const PAD_RIGHT: f64 = 34.0;
    const PAD_TOP: f64 = 24.0;
    const PAD_BOTTOM: f64 = 42.0;

    let inner_width = WIDTH - PAD_LEFT - PAD_RIGHT;
    let inner_height = HEIGHT - PAD_TOP - PAD_BOTTOM;
    let denom = trends.len().saturating_sub(1).max(1) as f64;
    let points = trends
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            let value = selector(point)?;
            let clamped = value.min(scale_max);
            let x = PAD_LEFT + inner_width * index as f64 / denom;
            let y = PAD_TOP + inner_height * (1.0 - clamped as f64 / scale_max as f64);
            Some((x, y))
        })
        .collect::<Vec<_>>();
    let path = svg_path(&points);
    let area = svg_area_path(&points, HEIGHT - PAD_BOTTOM);
    let grid = svg_grid_lines(
        scale_max,
        unit,
        PAD_LEFT,
        WIDTH - PAD_RIGHT,
        PAD_TOP,
        inner_height,
    );
    let ticks = svg_time_ticks(trends, PAD_LEFT, inner_width, HEIGHT - 15.0);
    let gradient_id = format!("nodelite-trend-{id_suffix}");

    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" viewBox="0 0 640 196" role="img" aria-label="{} trend" style="display:block;">"#,
            r#"<defs><linearGradient id="{}" x1="0" y1="0" x2="0" y2="1">"#,
            r#"<stop offset="0%" stop-color="{}" stop-opacity="0.55"/><stop offset="100%" stop-color="{}" stop-opacity="0"/></linearGradient></defs>"#,
            "{}{}",
            r#"<path d="{}" fill="url(#{})"/>"#,
            r#"<path d="{}" fill="none" stroke="{}" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round"/>"#,
            r#"</svg>"#
        ),
        escape_html(id_suffix),
        escape_html(&gradient_id),
        fill,
        fill,
        grid,
        ticks,
        area,
        escape_html(&gradient_id),
        path,
        color,
    )
}

fn svg_grid_lines(
    scale_max: u64,
    unit: &str,
    x1: f64,
    x2: f64,
    y1: f64,
    inner_height: f64,
) -> String {
    (0..=4)
        .map(|index| {
            let y = y1 + inner_height * index as f64 / 4.0;
            let value = scale_max.saturating_sub(scale_max.saturating_mul(index) / 4);
            format!(
                concat!(
                    r##"<line x1="{:.1}" x2="{:.1}" y1="{:.1}" y2="{:.1}" stroke="#111111" stroke-opacity="0.09" stroke-width="1"/>"##,
                    r##"<text x="{:.1}" y="{:.1}" text-anchor="end" fill="#6b6b6b" opacity="0.72" font-size="11" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif">{}</text>"##
                ),
                x1,
                x2,
                y,
                y,
                x1 - 8.0,
                y + 3.0,
                escape_html(&format_metric_value(value, unit)),
            )
        })
        .collect::<String>()
}

fn svg_time_ticks(trends: &[InspectionTrendPoint], x1: f64, inner_width: f64, y: f64) -> String {
    if trends.is_empty() {
        return String::new();
    }
    let last = trends.len().saturating_sub(1);
    let middle = last / 2;
    [(0, x1), (middle, x1 + inner_width / 2.0), (last, x1 + inner_width)]
        .into_iter()
        .map(|(index, x)| {
            let label = trends
                .get(index)
                .map(|point| point.label.as_str())
                .unwrap_or_default();
            format!(
                r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="#6b6b6b" opacity="0.62" font-size="10" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif">{}</text>"##,
                x,
                y,
                escape_html(label),
            )
        })
        .collect::<String>()
}

fn svg_path(points: &[(f64, f64)]) -> String {
    points
        .iter()
        .enumerate()
        .map(|(index, (x, y))| {
            if index == 0 {
                format!("M {:.1} {:.1}", x, y)
            } else {
                format!(" L {:.1} {:.1}", x, y)
            }
        })
        .collect::<String>()
}

fn svg_area_path(points: &[(f64, f64)], baseline: f64) -> String {
    let Some((first_x, _)) = points.first() else {
        return String::new();
    };
    let Some((last_x, _)) = points.last() else {
        return String::new();
    };
    let line = points
        .iter()
        .map(|(x, y)| format!(" L {:.1} {:.1}", x, y))
        .collect::<String>();
    format!(
        "M {:.1} {:.1}{} L {:.1} {:.1} L {:.1} {:.1} Z",
        first_x, baseline, line, last_x, baseline, first_x, baseline
    )
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TrendStats {
    pub(super) latest: u64,
    pub(super) average: u64,
    pub(super) peak: u64,
}

pub(super) fn trend_stats<F>(trends: &[InspectionTrendPoint], selector: F) -> Option<TrendStats>
where
    F: Fn(&InspectionTrendPoint) -> Option<u64> + Copy,
{
    let values = trends.iter().filter_map(selector).collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    let latest = trends
        .iter()
        .rev()
        .find_map(selector)
        .expect("non-empty values should have latest value");
    let sum = values
        .iter()
        .fold(0_u128, |sum, value| sum + *value as u128);
    let average = ((sum as f64) / values.len() as f64).round() as u64;
    let peak = values.iter().copied().max().unwrap_or(latest);
    Some(TrendStats {
        latest,
        average,
        peak,
    })
}

pub(super) fn trend_has_samples(trends: &[InspectionTrendPoint]) -> bool {
    trends.iter().any(|point| {
        point.cpu_usage_percent.is_some()
            || point.memory_used_percent.is_some()
            || point.latency_ms.is_some()
    })
}

fn trend_scale_max(stats: Option<TrendStats>, fixed_scale_max: Option<u64>) -> u64 {
    if let Some(max) = fixed_scale_max {
        return max.max(1);
    }
    let peak = stats.map(|stats| stats.peak).unwrap_or(1).max(1);
    let step = if peak <= 100 {
        25
    } else if peak <= 500 {
        50
    } else {
        100
    };
    peak.div_ceil(step) * step
}

pub(super) fn format_metric_value(value: u64, unit: &str) -> String {
    if unit == "%" {
        format!("{value}%")
    } else {
        format!("{value} {unit}")
    }
}
