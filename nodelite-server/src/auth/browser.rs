//! Long-lived browser connections inherit both credential and individual session lifetimes.

use std::time::Instant;

use axum::http::{HeaderMap, header};
use tokio_util::sync::CancellationToken;

use super::{
    BASIC_AUTH_SESSION_COOKIE, ReadonlyRouteAuth, TWO_FACTOR_AUTH_COOKIE, TwoFactorSessions,
    constant_time_compare_bytes, cookie_value,
};

#[derive(Debug, Clone)]
pub(crate) struct BrowserSession {
    pub(super) expires_at: Instant,
    revoked: CancellationToken,
}

impl BrowserSession {
    pub(super) fn new(expires_at: Instant) -> Self {
        Self {
            expires_at,
            revoked: CancellationToken::new(),
        }
    }

    pub(super) fn revoke(&self) {
        self.revoked.cancel();
    }

    async fn ended(&self) {
        tokio::select! {
            biased;
            _ = self.revoked.cancelled() => {}
            _ = tokio::time::sleep_until(self.expires_at.into()) => {}
        }
    }
}

pub(crate) struct BrowserAuthorization {
    credentials_revoked: CancellationToken,
    session: Option<BrowserSession>,
}

impl BrowserAuthorization {
    /// Rechecking under the current auth lock closes the middleware/upgrade race.
    pub(crate) fn capture(
        auth: &ReadonlyRouteAuth,
        sessions: &TwoFactorSessions,
        headers: &HeaderMap,
        created_session: Option<BrowserSession>,
    ) -> Option<Self> {
        let session = match auth.expected_authorization.as_deref() {
            None => None,
            Some(_) if auth.enable_2fa => {
                let token = cookie_value(headers, TWO_FACTOR_AUTH_COOKIE)?;
                Some(sessions.authenticated_lifetime(&token)?)
            }
            Some(expected) => {
                let supplied = headers.get(header::AUTHORIZATION)?.as_bytes();
                if !constant_time_compare_bytes(supplied, expected.as_bytes()) {
                    return None;
                }
                Some(created_session.or_else(|| {
                    let token = cookie_value(headers, BASIC_AUTH_SESSION_COOKIE)?;
                    sessions.basic_auth_lifetime(&token)
                })?)
            }
        };
        Some(Self {
            credentials_revoked: auth.revoked.clone(),
            session,
        })
    }

    pub(crate) async fn ended(&self) {
        tokio::select! {
            biased;
            _ = self.credentials_revoked.cancelled() => {}
            _ = async {
                if let Some(session) = &self.session {
                    session.ended().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }
}

#[cfg(test)]
impl TwoFactorSessions {
    pub(crate) fn expire_authenticated_after(&self, token: &str, duration: std::time::Duration) {
        let mut store = super::lock_mutex(&self.inner);
        store
            .authenticated
            .get_mut(token)
            .expect("existing test session")
            .lifetime
            .expires_at = Instant::now() + duration;
    }
}
