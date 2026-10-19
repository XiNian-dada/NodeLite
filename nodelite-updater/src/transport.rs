//! Fixed-origin HTTPS transport with bounded bodies, total deadlines and no redirects.

use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use tough::{Transport, TransportError, TransportErrorKind, TransportStream};
use url::Url;

use crate::error::{Result, UpdateError};
use crate::fs::PAYLOAD_LIMIT;
use crate::policy::Policy;

#[derive(Clone)]
pub(crate) struct Http {
    client: reqwest::Client,
    metadata: Url,
    targets: Url,
    deadline: Instant,
    pub failed: Arc<AtomicBool>,
}

impl fmt::Debug for Http {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FixedRepositoryTransport")
    }
}

impl Http {
    pub(crate) fn new(policy: &Policy) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            metadata: policy.metadata_url.clone(),
            targets: policy.targets_url.clone(),
            deadline: Instant::now() + Duration::from_secs(45),
            failed: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn bytes(&self, url: &Url) -> std::result::Result<Vec<u8>, TransportError> {
        let limit = if contained(&self.metadata, url) {
            2 * 1024 * 1024
        } else if contained(&self.targets, url) {
            PAYLOAD_LIMIT
        } else {
            return Err(TransportError::new(
                TransportErrorKind::UnsupportedUrlScheme,
                url,
            ));
        };
        let remaining = self
            .deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(10));
        if remaining.is_zero() {
            return Err(TransportError::new(TransportErrorKind::Other, url));
        }
        let response = self
            .client
            .get(url.clone())
            .timeout(remaining)
            .send()
            .await
            .map_err(|_| TransportError::new(TransportErrorKind::Other, url))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(TransportError::new(TransportErrorKind::FileNotFound, url));
        }
        if !response.status().is_success()
            || response.content_length().is_some_and(|n| n > limit as u64)
        {
            return Err(TransportError::new(TransportErrorKind::Other, url));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| TransportError::new(TransportErrorKind::Other, url))?;
            if chunk.len() > limit - bytes.len() {
                return Err(TransportError::new(TransportErrorKind::Other, url));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn contained(base: &Url, request: &Url) -> bool {
    base.origin() == request.origin()
        && request.path().starts_with(base.path())
        && request.query().is_none()
        && request.fragment().is_none()
        && request.username().is_empty()
        && request.password().is_none()
}

#[async_trait]
impl Transport for Http {
    async fn fetch(&self, url: Url) -> std::result::Result<TransportStream, TransportError> {
        match self.bytes(&url).await {
            Ok(bytes) => Ok(Box::pin(stream::once(std::future::ready(Ok(bytes.into()))))),
            Err(error) => {
                // Some clients treat every failed root fetch as a missing next root.
                // Never silently accept 503, timeouts or oversized root metadata that way.
                if error.kind() != TransportErrorKind::FileNotFound {
                    self.failed.store(true, Ordering::SeqCst);
                }
                Err(error)
            }
        }
    }
}

pub(crate) fn check_transport(http: &Http) -> Result<()> {
    if http.failed.load(Ordering::SeqCst) {
        return Err(UpdateError::Policy("repository transport failed closed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_scope_cannot_escape_origin_or_path() {
        let base = Url::parse("https://example.invalid/repo/metadata/").expect("base");
        for path in [
            "https://evil.invalid/repo/metadata/root.json",
            "http://example.invalid/repo/metadata/root.json",
            "https://example.invalid/repo/other/root.json",
            "https://example.invalid/repo/metadata/root.json?q=1",
        ] {
            assert!(!contained(&base, &Url::parse(path).expect("URL")));
        }
        assert!(contained(&base, &base.join("1.root.json").expect("join")));
    }
}
