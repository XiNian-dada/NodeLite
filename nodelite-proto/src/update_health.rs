//! Local, credential-free update evidence; never accepted as a remote update command.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateHealth {
    pub schema: u32,
    pub pid: u32,
    pub invocation: String,
    pub executable_sha256: String,
    pub sample_boottime_ms: u64,
    pub collected_at_ms: u64,
    pub sample_counter: u64,
}

impl UpdateHealth {
    pub fn validate(&self) -> bool {
        self.schema == 1
            && self.pid > 0
            && self.sample_counter > 0
            && self.sample_boottime_ms > 0
            && self.collected_at_ms > 0
            && self.invocation.len() == 32
            && self.executable_sha256.len() == 64
            && self
                .invocation
                .bytes()
                .chain(self.executable_sha256.bytes())
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_evidence_and_unknown_fields_are_rejected() {
        let mut health = UpdateHealth {
            schema: 1,
            pid: 1,
            invocation: "a".repeat(32),
            executable_sha256: "b".repeat(64),
            sample_boottime_ms: 1,
            collected_at_ms: 1,
            sample_counter: 1,
        };
        assert!(health.validate());
        health.invocation = "../escape".into();
        assert!(!health.validate());
        assert!(serde_json::from_str::<UpdateHealth>("{\"schema\":1,\"command\":\"sh\"}").is_err());
    }
}
