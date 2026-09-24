//! Tracks a brief confirmation grant on existing browser sessions.

use std::time::{Duration, Instant};

use super::{SENSITIVE_CONFIRMATION_SECS, TwoFactorSessions, lock_mutex, prune_expired_sessions};

impl TwoFactorSessions {
    pub(crate) fn confirm_sensitive_action(&self, token: &str, two_factor: bool) -> bool {
        let now = Instant::now();
        let mut store = lock_mutex(&self.inner);
        prune_expired_sessions(&mut store, now);
        let session = if two_factor {
            store
                .authenticated
                .get_mut(token)
                .map(|entry| &mut entry.confirmed_until)
        } else {
            store
                .basic_auth_sessions
                .get_mut(token)
                .map(|entry| &mut entry.confirmed_until)
        };
        if let Some(confirmed_until) = session {
            *confirmed_until = Some(now + Duration::from_secs(SENSITIVE_CONFIRMATION_SECS));
            true
        } else {
            false
        }
    }

    pub(crate) fn sensitive_action_confirmed(&self, token: &str, two_factor: bool) -> bool {
        let now = Instant::now();
        let mut store = lock_mutex(&self.inner);
        prune_expired_sessions(&mut store, now);
        if two_factor {
            store
                .authenticated
                .get(token)
                .and_then(|entry| entry.confirmed_until)
                .is_some_and(|until| until > now)
        } else {
            store
                .basic_auth_sessions
                .get(token)
                .and_then(|entry| entry.confirmed_until)
                .is_some_and(|until| until > now)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_confirmation_is_session_bound_and_expires() {
        let sessions = TwoFactorSessions::new();
        let first = sessions.create_authenticated().expect("first session");
        let second = sessions.create_authenticated().expect("second session");
        assert!(sessions.sensitive_action_confirmed(&first, true));
        assert!(sessions.sensitive_action_confirmed(&second, true));
        let basic = sessions
            .create_basic_auth_session(None)
            .expect("basic session");
        assert!(!sessions.sensitive_action_confirmed(&basic, false));
        {
            let mut store = lock_mutex(&sessions.inner);
            store
                .authenticated
                .get_mut(&second)
                .expect("second session")
                .confirmed_until = Some(Instant::now() - Duration::from_secs(1));
        }
        assert!(sessions.confirm_sensitive_action(&first, true));
        assert!(sessions.sensitive_action_confirmed(&first, true));
        assert!(!sessions.sensitive_action_confirmed(&second, true));
        assert!(!sessions.sensitive_action_confirmed(&first, false));
        {
            let mut store = lock_mutex(&sessions.inner);
            store
                .authenticated
                .get_mut(&first)
                .expect("session")
                .confirmed_until = Some(Instant::now() - Duration::from_secs(1));
        }
        assert!(!sessions.sensitive_action_confirmed(&first, true));
        sessions.clear_authenticated();
        assert!(!sessions.confirm_sensitive_action(&first, true));
    }
}
