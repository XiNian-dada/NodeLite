//! Only a root-owned local policy may choose repositories, channel and initial trust.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde::Deserialize;
use url::Url;

use crate::defaults::{disabled, stability, stable, timeout};
use crate::error::{Result, UpdateError};

pub(crate) const POLICY: &str = "/etc/nodelite-updater/policy.toml";
pub(crate) const ROOT: &str = "/etc/nodelite-updater/root.json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    #[serde(default = "disabled")]
    pub enabled: bool,
    #[serde(default = "disabled")]
    pub allow_remote_updates: bool,
    pub metadata_url: Url,
    pub targets_url: Url,
    #[serde(default = "stable")]
    pub channel: String,
    pub target: String,
    pub baseline_sha256: String,
    pub baseline_sequence: u64,
    pub minimum_sequence: u64,
    #[serde(default = "timeout")]
    pub trial_seconds: u64,
    #[serde(default = "stability")]
    pub stability_seconds: u64,
}

pub(crate) fn sanitize_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(UpdateError::Policy("invalid SHA256"));
    }
    Ok(())
}

pub(crate) fn sanitize_sequence(value: u64) -> Result<()> {
    if value == 0 || value >= 1 << 53 {
        return Err(UpdateError::Policy("invalid release sequence"));
    }
    Ok(())
}

pub(crate) fn trusted_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(UpdateError::Policy("trust path must be absolute"));
    }
    for ancestor in path.ancestors() {
        let info = std::fs::symlink_metadata(ancestor)?;
        if info.file_type().is_symlink() || info.uid() != 0 || info.mode() & 0o022 != 0 {
            return Err(UpdateError::Policy(
                "trust path must be root-owned and not writable by others",
            ));
        }
    }
    crate::fs::read(path, limit)
}

impl Policy {
    pub(crate) fn load() -> Result<Self> {
        let bytes = trusted_file(Path::new(POLICY), 16384)?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| UpdateError::Policy("non-UTF8 policy"))?;
        let policy: Self = toml::from_str(text)?;
        policy.validate()?;
        Ok(policy)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Err(UpdateError::Policy(
                "automatic updates are not explicitly enabled",
            ));
        }
        if !matches!(self.channel.as_str(), "stable" | "testing") {
            return Err(UpdateError::Policy("unsupported update channel"));
        }
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64-unknown-linux-musl",
            "aarch64" => "aarch64-unknown-linux-musl",
            _ => return Err(UpdateError::Policy("unsupported platform")),
        };
        if std::env::consts::OS != "linux" || self.target != arch {
            return Err(UpdateError::Policy("unsupported platform"));
        }
        for url in [&self.metadata_url, &self.targets_url] {
            let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
            if !(url.scheme() == "https" || url.scheme() == "http" && local)
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.path().ends_with('/')
            {
                return Err(UpdateError::Policy("repository URL rejected"));
            }
        }
        sanitize_digest(&self.baseline_sha256)?;
        sanitize_sequence(self.baseline_sequence)?;
        sanitize_sequence(self.minimum_sequence)?;
        if self.baseline_sequence < self.minimum_sequence
            || !(5..=60).contains(&self.stability_seconds)
            || self.trial_seconds <= self.stability_seconds
            || self.trial_seconds > 300
        {
            return Err(UpdateError::Policy(
                "invalid rollback floor or health deadline",
            ));
        }
        Ok(())
    }

    pub(crate) fn target_name(&self) -> String {
        format!("nodelite-agent-{}-{}", self.channel, self.target)
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> Policy {
    let mut policy: Policy =
        toml::from_str(include_str!("../policy.example.toml")).expect("example policy");
    policy.enabled = true;
    policy.baseline_sha256 = crate::fs::digest(b"baseline");
    policy.target = format!("{}-unknown-linux-musl", std::env::consts::ARCH);
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_and_sequence_bounds_are_strict() {
        for bad in ["", "../escape", &"F".repeat(64), &"0".repeat(63)] {
            assert!(sanitize_digest(bad).is_err());
        }
        assert!(sanitize_digest(&"f".repeat(64)).is_ok());
        for bad in [0, 1 << 53, u64::MAX] {
            assert!(sanitize_sequence(bad).is_err());
        }
        assert!(sanitize_sequence(1).is_ok());
    }

    #[test]
    fn example_policy_is_disabled_and_rejects_unknown_fields() {
        let example = include_str!("../policy.example.toml");
        let policy: Policy = toml::from_str(example).expect("example policy");
        assert!(!policy.enabled);
        assert!(policy.validate().is_err());
        assert!(toml::from_str::<Policy>(&format!("{example}\ncommand = 'sh'\n")).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn unsafe_urls_channels_and_deadlines_are_rejected() {
        let mut policy = fixture();
        assert!(policy.validate().is_ok());
        for url in [
            "file:///etc/passwd",
            "http://example.invalid/repo/",
            "https://user:password@example.invalid/repo/",
            "https://example.invalid/repo/?token=x",
        ] {
            policy.metadata_url = Url::parse(url).expect("URL");
            assert!(policy.validate().is_err(), "{url}");
        }
        policy = fixture();
        policy.channel = "../stable".into();
        assert!(policy.validate().is_err());
        policy = fixture();
        policy.trial_seconds = policy.stability_seconds;
        assert!(policy.validate().is_err());
    }
}
