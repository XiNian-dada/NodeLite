//! Session-scoped Agent capability reports never outlive their authenticated connection.

use std::sync::Arc;

use dashmap::DashMap;
use nodelite_proto::TrafficControlStatus;

#[derive(Clone, Default)]
pub(crate) struct TrafficControlStatuses {
    sessions: Arc<DashMap<String, (u64, Option<TrafficControlStatus>)>>,
}

impl TrafficControlStatuses {
    pub(crate) fn begin(&self, node_id: &str, session_id: u64) {
        let mut entry = self
            .sessions
            .entry(node_id.to_string())
            .or_insert((session_id, None));
        if session_id > entry.0 {
            *entry = (session_id, None);
        }
    }

    pub(crate) fn record(&self, node_id: &str, session_id: u64, status: TrafficControlStatus) {
        if let Some(mut entry) = self.sessions.get_mut(node_id)
            && entry.0 == session_id
        {
            entry.1 = Some(status);
        }
    }

    pub(crate) fn get(&self, node_id: &str) -> Option<TrafficControlStatus> {
        self.sessions.get(node_id).and_then(|entry| entry.1.clone())
    }

    pub(crate) fn end(&self, node_id: &str, session_id: u64) {
        self.sessions
            .remove_if(node_id, |_, entry| entry.0 == session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodelite_proto::{TrafficControlState, TrafficControlUnavailableReason};

    #[test]
    fn old_sessions_cannot_overwrite_or_remove_the_current_capability_report() {
        let statuses = TrafficControlStatuses::default();
        let status = TrafficControlStatus {
            state: TrafficControlState::Unavailable,
            reason: Some(TrafficControlUnavailableReason::MissingTc),
            desired_rate_kbps: Some(1000),
            applied_rate_kbps: None,
        };
        statuses.begin("node", 1);
        statuses.begin("node", 2);
        statuses.record("node", 2, status.clone());
        statuses.begin("node", 1);
        statuses.end("node", 1);
        assert_eq!(statuses.get("node"), Some(status));
        statuses.end("node", 2);
        assert_eq!(statuses.get("node"), None);
    }
}
