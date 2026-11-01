//! Session capacity is shared, race-safe, and never evicts an authenticated browser.

use super::*;

fn session_count(sessions: &TwoFactorSessions) -> usize {
    let store = lock_mutex(&sessions.inner);
    store.authenticated.len() + store.basic_auth_sessions.len()
}

fn pending_count(sessions: &TwoFactorSessions) -> usize {
    lock_mutex(&sessions.inner).pending.len()
}

#[test]
fn concurrent_session_creation_cannot_exceed_the_shared_budget() {
    let sessions = TwoFactorSessions::new();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let sessions = sessions.clone();
            std::thread::spawn(move || {
                (0..MAX_AUTH_SESSIONS / 4)
                    .filter(|_| match sessions.create_basic_auth_session(None) {
                        Ok(_) => true,
                        Err(AuthSessionError::Capacity) => false,
                        Err(error) => panic!("unexpected session error: {error}"),
                    })
                    .count()
            })
        })
        .collect();
    let successes: usize = handles
        .into_iter()
        .map(|handle| handle.join().expect("creator thread"))
        .sum();
    assert_eq!(successes, MAX_AUTH_SESSIONS);
    assert_eq!(session_count(&sessions), MAX_AUTH_SESSIONS);
}

#[test]
fn full_store_preserves_existing_sessions_and_allows_pending_exchanges() {
    let sessions = TwoFactorSessions::new();
    let basic = sessions.create_basic_auth_session(Some(42)).expect("basic");
    assert!(sessions.confirm_sensitive_action(&basic, false));
    let totp_pending = sessions.create_pending().expect("TOTP pending");
    let passkey_pending = sessions.create_pending().expect("Passkey pending");
    for _ in 1..MAX_AUTH_SESSIONS {
        sessions.create_authenticated().expect("fill store");
    }
    assert!(matches!(
        sessions.create_authenticated(),
        Err(AuthSessionError::Capacity)
    ));
    assert!(matches!(
        sessions.create_basic_auth_session(None),
        Err(AuthSessionError::Capacity)
    ));
    assert!(sessions.is_basic_auth_session_valid(&basic));
    assert_eq!(sessions.get_basic_auth_login_event_id(&basic), Some(42));
    assert!(sessions.sensitive_action_confirmed(&basic, false));
    sessions
        .create_pending()
        .expect("pending sessions do not use the shared budget");
    let totp = sessions
        .exchange_pending(&totp_pending, &[123])
        .expect("exchange TOTP")
        .expect("authenticated TOTP");
    let passkey = sessions
        .exchange_pending_for_passkey(&passkey_pending)
        .expect("exchange Passkey")
        .expect("authenticated Passkey");
    assert!(sessions.is_authenticated(&totp));
    assert!(sessions.is_authenticated(&passkey));
    sessions.remove_basic_auth_session(&basic);
    assert!(!sessions.is_basic_auth_session_valid(&basic));
    sessions.clear_authenticated();
    assert_eq!(session_count(&sessions), 0);
}

#[test]
fn pending_sessions_have_their_own_budget_and_evict_the_oldest() {
    let sessions = TwoFactorSessions::new();
    let oldest = sessions.create_pending().expect("first pending");
    std::thread::sleep(std::time::Duration::from_millis(2));
    let newer = sessions.create_pending().expect("second pending");
    for _ in 2..MAX_PENDING_SESSIONS {
        sessions.create_pending().expect("fill pending budget");
    }
    assert_eq!(pending_count(&sessions), MAX_PENDING_SESSIONS);

    let latest = sessions
        .create_pending()
        .expect("a new login always gets a pending slot");
    assert_eq!(pending_count(&sessions), MAX_PENDING_SESSIONS);
    assert!(
        !sessions.pending_exists(&oldest),
        "the oldest pending is evicted"
    );
    assert!(sessions.pending_exists(&newer));
    assert!(sessions.pending_exists(&latest));
    assert_eq!(session_count(&sessions), 0);
    sessions
        .create_authenticated()
        .expect("pending sessions never block authenticated ones");
}

#[test]
fn expired_entries_free_capacity_without_expanding_the_cleanup_budget() {
    let sessions = TwoFactorSessions::new();
    let before = Instant::now();
    let basic = sessions.create_basic_auth_session(Some(42)).expect("basic");
    let lifetime = sessions.basic_auth_lifetime(&basic).expect("lifetime");
    assert!(lifetime.expires_at >= before + Duration::from_secs(BASIC_AUTH_SESSION_SECS));
    assert!(lifetime.expires_at <= Instant::now() + Duration::from_secs(BASIC_AUTH_SESSION_SECS));
    for _ in 1..MAX_AUTH_SESSIONS {
        sessions.create_authenticated().expect("fill store");
    }
    {
        let mut store = lock_mutex(&sessions.inner);
        store
            .basic_auth_sessions
            .get_mut(&basic)
            .expect("basic")
            .lifetime
            .expires_at = Instant::now() - Duration::from_secs(1);
    }
    assert!(!sessions.is_basic_auth_session_valid(&basic));
    assert!(sessions.basic_auth_lifetime(&basic).is_none());
    assert!(sessions.get_basic_auth_login_event_id(&basic).is_none());
    assert!(sessions.bind_basic_auth_login(&basic, Some(99)).is_none());
    assert_eq!(session_count(&sessions), MAX_AUTH_SESSIONS);
    sessions.create_authenticated().expect("reuse expired slot");
    assert_eq!(session_count(&sessions), MAX_AUTH_SESSIONS);
    assert!(
        !lock_mutex(&sessions.inner)
            .basic_auth_sessions
            .contains_key(&basic)
    );
}
