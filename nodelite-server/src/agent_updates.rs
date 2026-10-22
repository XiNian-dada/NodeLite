//! Bounded, session-bound observations; only the node can confirm activation or rollback.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nodelite_proto::agent_update::{
    AgentUpdateCapability as Capability, AgentUpdatePhase as Phase, AgentUpdateReason as Reason,
    AgentUpdateRequest, AgentUpdateStatus, UPDATE_COOLDOWN_SECS,
};

const CAPACITY: usize = 4096;

#[derive(Clone, Default)]
pub(crate) struct AgentUpdates(Arc<Mutex<HashMap<String, Entry>>>);

struct Entry {
    session_id: u64,
    online: bool,
    reported: bool,
    touched: Instant,
    requested: Option<Instant>,
    request: Option<AgentUpdateRequest>,
    status: AgentUpdateStatus,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RequestError {
    #[error("agent has not enabled remote signed updates")]
    Unavailable,
    #[error("agent update is already pending")]
    Busy,
    #[error("agent update requests require a five-minute cooldown")]
    RateLimited,
}

impl AgentUpdates {
    pub(crate) fn begin(&self, node_id: &str, session_id: u64) {
        let Ok(mut entries) = self.0.lock() else {
            return;
        };
        entries
            .retain(|_, entry| entry.online || entry.touched.elapsed() < Duration::from_secs(3600));
        if !entries.contains_key(node_id) && entries.len() >= CAPACITY {
            return;
        }
        let entry = entries.entry(node_id.into()).or_insert_with(|| Entry {
            session_id: 0,
            online: false,
            reported: false,
            touched: Instant::now(),
            requested: None,
            request: None,
            status: AgentUpdateStatus::unavailable(Capability::Disabled, Reason::NotEnrolled),
        });
        if session_id > entry.session_id {
            entry.session_id = session_id;
            entry.online = true;
            entry.reported = false;
            entry.touched = Instant::now();
        }
    }

    pub(crate) fn end(&self, node_id: &str, session_id: u64) {
        if let Ok(mut entries) = self.0.lock()
            && let Some(entry) = entries.get_mut(node_id)
            && entry.session_id == session_id
        {
            entry.online = false;
            entry.touched = Instant::now();
        }
    }

    pub(crate) fn record(&self, node_id: &str, session_id: u64, status: AgentUpdateStatus) {
        if let Ok(mut entries) = self.0.lock()
            && let Some(entry) = entries.get_mut(node_id)
            && entry.session_id == session_id
            && entry.online
        {
            entry.reported = true;
            entry.touched = Instant::now();
            let expected = entry
                .request
                .as_ref()
                .map(|request| request.request_id.as_str());
            if expected.is_none() || expected == status.request_id.as_deref() {
                entry.status = status;
            } else {
                entry.status.capability = status.capability;
                if status.capability == Capability::ManualRecovery {
                    entry.status.phase = Phase::ManualRecovery;
                }
                if status.capability != Capability::Ready {
                    entry.status.reason = status.reason;
                }
            }
        }
    }

    pub(crate) fn view(&self, node_id: &str, now: u64) -> (AgentUpdateStatus, bool) {
        let unavailable = || {
            (
                AgentUpdateStatus::unavailable(Capability::Disabled, Reason::NotEnrolled),
                false,
            )
        };
        let Ok(entries) = self.0.lock() else {
            return unavailable();
        };
        let Some(entry) = entries.get(node_id) else {
            return unavailable();
        };
        let mut status = entry.status.clone();
        if status.phase == Phase::Queued
            && entry
                .request
                .as_ref()
                .is_some_and(|request| request.expires_at <= now)
        {
            status.phase = Phase::Unknown;
            status.reason = Some(Reason::RequestExpired);
        }
        if !entry.reported {
            status.capability = Capability::Disabled;
        }
        let cooldown = entry.requested.is_none_or(|requested| {
            requested.elapsed() >= Duration::from_secs(UPDATE_COOLDOWN_SECS)
        });
        let ready = entry.online
            && entry.reported
            && cooldown
            && status.capability == Capability::Ready
            && !status.phase.in_progress();
        (status, ready)
    }

    pub(crate) fn reserve(
        &self,
        node_id: &str,
        request: AgentUpdateRequest,
    ) -> Result<(), RequestError> {
        let mut entries = self.0.lock().map_err(|_| RequestError::Unavailable)?;
        let entry = entries.get_mut(node_id).ok_or(RequestError::Unavailable)?;
        if entry.status.phase == Phase::Queued
            && entry
                .request
                .as_ref()
                .is_some_and(|previous| previous.expires_at <= request.issued_at)
        {
            entry.status.phase = Phase::Unknown;
            entry.status.reason = Some(Reason::RequestExpired);
        }
        if !entry.online || !entry.reported || entry.status.capability != Capability::Ready {
            return Err(RequestError::Unavailable);
        }
        if entry.status.phase.in_progress() {
            return Err(RequestError::Busy);
        }
        if entry
            .requested
            .is_some_and(|when| when.elapsed() < Duration::from_secs(UPDATE_COOLDOWN_SECS))
        {
            return Err(RequestError::RateLimited);
        }
        entry.requested = Some(Instant::now());
        entry.touched = Instant::now();
        entry.status.phase = Phase::Queued;
        entry.status.request_id = Some(request.request_id.clone());
        entry.status.updated_at = request.issued_at;
        entry.status.reason = None;
        entry.request = Some(request);
        Ok(())
    }

    pub(crate) fn delivery_failed(&self, node_id: &str, request_id: &str) {
        if let Ok(mut entries) = self.0.lock()
            && let Some(entry) = entries.get_mut(node_id)
            && entry.status.request_id.as_deref() == Some(request_id)
        {
            entry.status.phase = Phase::Failed;
            entry.status.reason = Some(Reason::UpdateFailed);
        }
    }

    pub(crate) fn forget(&self, node_id: &str) {
        if let Ok(mut entries) = self.0.lock() {
            entries.remove(node_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> AgentUpdateStatus {
        let mut status = AgentUpdateStatus::unavailable(Capability::Ready, Reason::NotEnrolled);
        status.reason = None;
        status
    }

    #[test]
    fn capability_requires_a_current_session_and_pending_jobs_survive_reconnect() {
        let updates = AgentUpdates::default();
        updates.begin("node", 1);
        assert!(!updates.view("node", 100).1);
        updates.record("node", 1, ready());
        assert!(updates.view("node", 100).1);
        let request = AgentUpdateRequest {
            request_id: "a".repeat(32),
            issued_at: 100,
            expires_at: 160,
        };
        updates.reserve("node", request.clone()).expect("request");
        assert!(matches!(
            updates.reserve("node", request),
            Err(RequestError::Busy)
        ));
        updates.end("node", 1);
        updates.begin("node", 2);
        updates.end("node", 1);
        updates.record("node", 1, ready());
        assert_eq!(updates.view("node", 100).0.phase, Phase::Queued);
        assert_eq!(updates.view("node", 160).0.phase, Phase::Unknown);
        let mut committed = ready();
        committed.request_id = Some("a".repeat(32));
        committed.phase = Phase::Committed;
        updates.record("node", 2, committed);
        assert_eq!(updates.view("node", 170).0.phase, Phase::Committed);
        assert!(!updates.view("node", 170).1);
    }

    #[test]
    fn unrelated_reports_cannot_claim_that_a_queued_request_succeeded() {
        let updates = AgentUpdates::default();
        updates.begin("node", 1);
        updates.record("node", 1, ready());
        updates
            .reserve(
                "node",
                AgentUpdateRequest {
                    request_id: "a".repeat(32),
                    issued_at: 100,
                    expires_at: 160,
                },
            )
            .expect("reserve");
        let mut old = ready();
        old.request_id = Some("b".repeat(32));
        old.phase = Phase::Committed;
        updates.record("node", 1, old);
        assert_eq!(updates.view("node", 100).0.phase, Phase::Queued);
        updates.forget("node");
        assert!(!updates.view("node", 100).1);
    }
}
