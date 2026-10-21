//! Bounded Basic-only browser sessions keep machine authentication stateless.

use std::time::{Duration, Instant};

use super::{
    AuthSessionResult, BASIC_AUTH_SESSION_SECS, BasicAuthSession, BrowserSession,
    TwoFactorSessions, ensure_session_capacity, generate_session_token, lock_mutex,
    prune_expired_sessions,
};

impl TwoFactorSessions {
    /// Associates a browser login with its audit event for the last-login API.
    pub fn create_basic_auth_session(
        &self,
        login_event_id: Option<i64>,
    ) -> AuthSessionResult<String> {
        let token = generate_session_token()?;
        let now = Instant::now();
        let mut store = lock_mutex(&self.inner);
        prune_expired_sessions(&mut store, now);
        ensure_session_capacity(&store)?;
        store.basic_auth_sessions.insert(
            token.clone(),
            BasicAuthSession {
                lifetime: BrowserSession::new(now + Duration::from_secs(BASIC_AUTH_SESSION_SECS)),
                login_event_id,
                confirmed_until: None,
            },
        );
        Ok(token)
    }

    pub fn is_basic_auth_session_valid(&self, token: &str) -> bool {
        let store = lock_mutex(&self.inner);
        store
            .basic_auth_sessions
            .get(token)
            .is_some_and(|session| session.lifetime.expires_at > Instant::now())
    }

    /// Excludes the current browser session from the previous-login result.
    pub fn get_basic_auth_login_event_id(&self, token: &str) -> Option<i64> {
        let store = lock_mutex(&self.inner);
        store
            .basic_auth_sessions
            .get(token)
            .filter(|session| session.lifetime.expires_at > Instant::now())
            .and_then(|session| session.login_event_id)
    }

    pub(crate) fn basic_auth_lifetime(&self, token: &str) -> Option<BrowserSession> {
        let store = lock_mutex(&self.inner);
        store
            .basic_auth_sessions
            .get(token)
            .filter(|session| session.lifetime.expires_at > Instant::now())
            .map(|session| session.lifetime.clone())
    }

    pub(crate) fn bind_basic_auth_login(
        &self,
        token: &str,
        login_event_id: Option<i64>,
    ) -> Option<BrowserSession> {
        let mut store = lock_mutex(&self.inner);
        let session = store
            .basic_auth_sessions
            .get_mut(token)
            .filter(|session| session.lifetime.expires_at > Instant::now())?;
        session.login_event_id = login_event_id;
        Some(session.lifetime.clone())
    }

    pub(crate) fn remove_basic_auth_session(&self, token: &str) {
        let mut store = lock_mutex(&self.inner);
        if let Some(session) = store.basic_auth_sessions.remove(token) {
            session.lifetime.revoke();
        }
    }
}
