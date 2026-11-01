//! WebAuthn 通行密钥（Passkey）状态管理、凭据持久化与挑战协商模块。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;
use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse, Url, Webauthn,
    WebauthnBuilder,
};

use crate::fs_security::{PrivateWriteError, atomic_write_private};

const PASSKEY_CEREMONY_SECS: u64 = 300;
const MAX_PASSKEYS: usize = 8;
const PASSKEY_LABEL_MAX_BYTES: usize = 64;

#[derive(Clone)]
pub(crate) struct PasskeyService {
    webauthn: Option<Arc<Webauthn>>,
    path: Arc<PathBuf>,
    credentials: Arc<Mutex<PasskeyFile>>,
    ceremonies: Arc<Mutex<PasskeyCeremonies>>,
    mutation_lock: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    before_write: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PasskeyError {
    #[error("passkeys require server.public_base_url to use https://")]
    InsecureOrigin,
    #[error("server.public_base_url is not a valid passkey origin")]
    InvalidOrigin,
    #[error("passkey storage read failed: {0}")]
    Read(#[source] std::io::Error),
    #[error("passkey storage is invalid: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("passkey storage write task failed: {0}")]
    WriteTask(#[from] tokio::task::JoinError),
    #[error("passkey storage write failed: {0}")]
    Write(#[from] PrivateWriteError),
    #[error("passkey service state is unavailable")]
    StateUnavailable,
    #[error("passkey operation failed")]
    Operation,
    #[error("passkey registration has expired")]
    RegistrationExpired,
    #[error("passkey authentication has expired")]
    AuthenticationExpired,
    #[error("no passkeys are registered")]
    NoCredentials,
    #[error("the passkey is already registered")]
    DuplicateCredential,
    #[error("the maximum number of passkeys has been reached")]
    CredentialLimit,
    #[error("the requested passkey does not exist")]
    CredentialNotFound,
    #[error("passkey labels must contain 1 to {PASSKEY_LABEL_MAX_BYTES} non-control bytes")]
    InvalidLabel,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PasskeySummary {
    pub(crate) id: Uuid,
    pub(crate) label: String,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredPasskey {
    id: Uuid,
    label: String,
    created_at: DateTime<Utc>,
    passkey: Passkey,
}

#[derive(Clone, Serialize, Deserialize)]
struct PasskeyFile {
    account_id: Uuid,
    #[serde(default)]
    credentials: Vec<StoredPasskey>,
}

struct PasskeyCeremonies {
    registrations: HashMap<String, PendingRegistration>,
    authentications: HashMap<String, PendingAuthentication>,
}

struct PendingRegistration {
    expires_at: Instant,
    label: String,
    state: PasskeyRegistration,
}

struct PendingAuthentication {
    expires_at: Instant,
    state: PasskeyAuthentication,
}

impl PasskeyService {
    pub(crate) async fn load(
        config_path: &Path,
        public_base_url: &str,
    ) -> Result<Self, PasskeyError> {
        let path = passkey_storage_path(config_path);
        let credentials = match fs::read(&path).await {
            Ok(contents) => serde_json::from_slice(&contents)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PasskeyFile {
                account_id: Uuid::new_v4(),
                credentials: Vec::new(),
            },
            Err(error) => return Err(PasskeyError::Read(error)),
        };

        Ok(Self {
            webauthn: build_webauthn(public_base_url).ok().map(Arc::new),
            path: Arc::new(path),
            credentials: Arc::new(Mutex::new(credentials)),
            ceremonies: Arc::new(Mutex::new(PasskeyCeremonies {
                registrations: HashMap::new(),
                authentications: HashMap::new(),
            })),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            before_write: None,
        })
    }

    pub(crate) fn summaries(&self) -> Result<Vec<PasskeySummary>, PasskeyError> {
        let credentials = lock(&self.credentials)?;
        Ok(credentials
            .credentials
            .iter()
            .map(|credential| PasskeySummary {
                id: credential.id,
                label: credential.label.clone(),
                created_at: credential.created_at,
            })
            .collect())
    }

    pub(crate) fn start_registration(
        &self,
        username: &str,
        session_binding: &str,
        label: &str,
    ) -> Result<CreationChallengeResponse, PasskeyError> {
        let label = normalize_label(label)?;
        let webauthn = self.webauthn()?;
        let (account_id, exclude_credentials) = {
            let credentials = lock(&self.credentials)?;
            if credentials.credentials.len() >= MAX_PASSKEYS {
                return Err(PasskeyError::CredentialLimit);
            }
            let exclude_credentials = credentials
                .credentials
                .iter()
                .map(|credential| credential.passkey.cred_id().clone())
                .collect::<Vec<_>>();
            (credentials.account_id, exclude_credentials)
        };
        let (challenge, state) = webauthn
            .start_passkey_registration(
                account_id,
                username,
                username,
                (!exclude_credentials.is_empty()).then_some(exclude_credentials),
            )
            .map_err(|_| PasskeyError::Operation)?;
        let now = Instant::now();
        let mut ceremonies = lock(&self.ceremonies)?;
        ceremonies.prune(now);
        ceremonies.registrations.insert(
            session_binding.to_string(),
            PendingRegistration {
                expires_at: now + Duration::from_secs(PASSKEY_CEREMONY_SECS),
                label,
                state,
            },
        );
        Ok(challenge)
    }

    pub(crate) async fn finish_registration(
        &self,
        session_binding: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<PasskeySummary, PasskeyError> {
        let pending = self.take_registration(session_binding)?;
        let passkey = self
            .webauthn()?
            .finish_passkey_registration(response, &pending.state)
            .map_err(|_| PasskeyError::Operation)?;
        self.add_credential(pending.label, passkey).await
    }

    async fn add_credential(
        &self,
        label: String,
        passkey: Passkey,
    ) -> Result<PasskeySummary, PasskeyError> {
        self.commit(move |next| {
            if next.credentials.len() >= MAX_PASSKEYS {
                return Err(PasskeyError::CredentialLimit);
            }
            if next
                .credentials
                .iter()
                .any(|credential| credential.passkey.cred_id() == passkey.cred_id())
            {
                return Err(PasskeyError::DuplicateCredential);
            }
            let stored = StoredPasskey {
                id: Uuid::new_v4(),
                label,
                created_at: Utc::now(),
                passkey,
            };
            let summary = PasskeySummary {
                id: stored.id,
                label: stored.label.clone(),
                created_at: stored.created_at,
            };
            next.credentials.push(stored);
            Ok((summary, true))
        })
        .await
    }

    pub(crate) fn start_authentication(
        &self,
        session_binding: &str,
    ) -> Result<RequestChallengeResponse, PasskeyError> {
        let webauthn = self.webauthn()?;
        let passkeys = lock(&self.credentials)?
            .credentials
            .iter()
            .map(|credential| credential.passkey.clone())
            .collect::<Vec<_>>();
        if passkeys.is_empty() {
            return Err(PasskeyError::NoCredentials);
        }
        let (challenge, state) = webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(|_| PasskeyError::Operation)?;
        let now = Instant::now();
        let mut ceremonies = lock(&self.ceremonies)?;
        ceremonies.prune(now);
        ceremonies.authentications.insert(
            session_binding.to_string(),
            PendingAuthentication {
                expires_at: now + Duration::from_secs(PASSKEY_CEREMONY_SECS),
                state,
            },
        );
        Ok(challenge)
    }

    pub(crate) async fn finish_authentication(
        &self,
        session_binding: &str,
        response: &PublicKeyCredential,
    ) -> Result<(), PasskeyError> {
        let pending = self.take_authentication(session_binding)?;
        let result = self
            .webauthn()?
            .finish_passkey_authentication(response, &pending.state)
            .map_err(|_| PasskeyError::Operation)?;
        self.commit(move |next| {
            for credential in &mut next.credentials {
                if let Some(changed) = credential.passkey.update_credential(&result) {
                    return Ok(((), changed));
                }
            }
            Err(PasskeyError::CredentialNotFound)
        })
        .await
    }

    pub(crate) async fn delete(&self, id: Uuid) -> Result<(), PasskeyError> {
        self.commit(move |next| {
            let Some(index) = next
                .credentials
                .iter()
                .position(|credential| credential.id == id)
            else {
                return Err(PasskeyError::CredentialNotFound);
            };
            next.credentials.remove(index);
            Ok(((), true))
        })
        .await
    }

    /// Runs one credential mutation end to end in a detached task.
    ///
    /// The blocking write cannot be cancelled once started, so if the caller's
    /// future owned the lock and the in-memory publish, dropping it mid-write
    /// would release the lock with disk ahead of memory and let the next
    /// mutation rewrite stale state (e.g. resurrect a revoked passkey). Owning
    /// lock, write and publish in one task keeps them a single unit.
    async fn commit<R, F>(&self, mutate: F) -> Result<R, PasskeyError>
    where
        R: Send + 'static,
        F: FnOnce(&mut PasskeyFile) -> Result<(R, bool), PasskeyError> + Send + 'static,
    {
        let service = self.clone();
        tokio::spawn(async move {
            let _mutation_guard = service.mutation_lock.clone().lock_owned().await;
            let mut next = service.credentials_snapshot()?;
            let (output, changed) = mutate(&mut next)?;
            if changed {
                service.persist(&next).await?;
                *lock(&service.credentials)? = next;
            }
            Ok(output)
        })
        .await?
    }

    fn webauthn(&self) -> Result<&Webauthn, PasskeyError> {
        self.webauthn.as_deref().ok_or(PasskeyError::InsecureOrigin)
    }

    fn credentials_snapshot(&self) -> Result<PasskeyFile, PasskeyError> {
        Ok(lock(&self.credentials)?.clone())
    }

    fn take_registration(
        &self,
        session_binding: &str,
    ) -> Result<PendingRegistration, PasskeyError> {
        let now = Instant::now();
        let mut ceremonies = lock(&self.ceremonies)?;
        ceremonies.prune(now);
        ceremonies
            .registrations
            .remove(session_binding)
            .ok_or(PasskeyError::RegistrationExpired)
    }

    fn take_authentication(
        &self,
        session_binding: &str,
    ) -> Result<PendingAuthentication, PasskeyError> {
        let now = Instant::now();
        let mut ceremonies = lock(&self.ceremonies)?;
        ceremonies.prune(now);
        ceremonies
            .authentications
            .remove(session_binding)
            .ok_or(PasskeyError::AuthenticationExpired)
    }

    async fn persist(&self, credentials: &PasskeyFile) -> Result<(), PasskeyError> {
        let payload = serde_json::to_vec(credentials)?;
        let path = self.path.as_ref().clone();
        #[cfg(test)]
        let before_write = self.before_write.clone();
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(hook) = before_write {
                hook();
            }
            atomic_write_private(&path, &payload)
        })
        .await??;
        Ok(())
    }
}

impl PasskeyCeremonies {
    fn prune(&mut self, now: Instant) {
        self.registrations
            .retain(|_, pending| pending.expires_at > now);
        self.authentications
            .retain(|_, pending| pending.expires_at > now);
    }
}

pub(crate) fn passkey_storage_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("passkeys.json")
}

fn build_webauthn(public_base_url: &str) -> Result<Webauthn, PasskeyError> {
    let origin = Url::parse(public_base_url).map_err(|_| PasskeyError::InvalidOrigin)?;
    if origin.scheme() != "https" {
        return Err(PasskeyError::InsecureOrigin);
    }
    let rp_id = origin.host_str().ok_or(PasskeyError::InvalidOrigin)?;
    WebauthnBuilder::new(rp_id, &origin)
        .map_err(|_| PasskeyError::InvalidOrigin)?
        .rp_name("NodeLite")
        .build()
        .map_err(|_| PasskeyError::InvalidOrigin)
}

fn normalize_label(label: &str) -> Result<String, PasskeyError> {
    let label = label.trim();
    if label.is_empty()
        || label.len() > PASSKEY_LABEL_MAX_BYTES
        || label.chars().any(char::is_control)
    {
        return Err(PasskeyError::InvalidLabel);
    }
    Ok(label.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, PasskeyError> {
    mutex.lock().map_err(|_| PasskeyError::StateUnavailable)
}

#[cfg(test)]
#[path = "passkeys_cancel_tests.rs"]
mod cancel_tests;

#[cfg(test)]
mod tests {
    use super::{PasskeyError, build_webauthn, normalize_label, passkey_storage_path};
    use std::path::Path;

    #[test]
    fn passkey_storage_stays_next_to_server_config() {
        assert_eq!(
            passkey_storage_path(Path::new("/opt/nodelite/config/server.toml")),
            Path::new("/opt/nodelite/config/passkeys.json")
        );
    }

    #[test]
    fn passkeys_require_a_secure_public_origin() {
        assert!(matches!(
            build_webauthn("http://monitor.example.com"),
            Err(PasskeyError::InsecureOrigin)
        ));
        assert!(build_webauthn("https://monitor.example.com").is_ok());
    }

    #[test]
    fn labels_are_bounded_and_control_free() {
        assert!(matches!(
            normalize_label("  MacBook Pro  ").as_deref(),
            Ok("MacBook Pro")
        ));
        assert!(matches!(
            normalize_label(""),
            Err(PasskeyError::InvalidLabel)
        ));
        assert!(matches!(
            normalize_label("bad\nlabel"),
            Err(PasskeyError::InvalidLabel)
        ));
        assert!(matches!(
            normalize_label(&"x".repeat(65)),
            Err(PasskeyError::InvalidLabel)
        ));
    }
}
