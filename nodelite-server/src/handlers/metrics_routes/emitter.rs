use std::collections::HashSet;
use std::fmt::Write;

#[derive(Clone, Copy)]
enum MetricKind {
    Gauge,
    Counter,
}

impl MetricKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Gauge => "gauge",
            Self::Counter => "counter",
        }
    }
}

#[derive(Default)]
pub(super) struct MetricEmitter {
    body: String,
    seen_metric_families: HashSet<&'static str>,
}

impl MetricEmitter {
    pub(super) fn finish(self) -> String {
        self.body
    }

    pub(super) fn gauge<T: std::fmt::Display>(
        &mut self,
        name: &'static str,
        help: &'static str,
        labels: &[(&str, &str)],
        value: T,
    ) {
        self.metric(MetricKind::Gauge, name, help, labels, value);
    }

    pub(super) fn counter<T: std::fmt::Display>(
        &mut self,
        name: &'static str,
        help: &'static str,
        labels: &[(&str, &str)],
        value: T,
    ) {
        self.metric(MetricKind::Counter, name, help, labels, value);
    }

    fn metric<T: std::fmt::Display>(
        &mut self,
        kind: MetricKind,
        name: &'static str,
        help: &'static str,
        labels: &[(&str, &str)],
        value: T,
    ) {
        if self.seen_metric_families.insert(name) {
            let _ = writeln!(self.body, "# HELP {name} {help}");
            let _ = writeln!(self.body, "# TYPE {name} {}", kind.as_str());
        }
        self.body.push_str(name);
        if !labels.is_empty() {
            self.body.push('{');
            for (index, (key, raw_value)) in labels.iter().enumerate() {
                if index > 0 {
                    self.body.push(',');
                }
                let escaped = escape_prometheus_label_value(raw_value);
                let _ = write!(self.body, "{key}=\"{escaped}\"");
            }
            self.body.push('}');
        }
        let _ = writeln!(self.body, " {value}");
    }
}

fn escape_prometheus_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
