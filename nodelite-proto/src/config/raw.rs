//! Decode each peer's settings without compiling the other peer's configuration.

mod agent;
#[cfg(feature = "server-config")]
mod alerts;
#[cfg(feature = "server-config")]
mod server;

pub(super) use agent::RawAgentConfigFile;
#[cfg(feature = "server-config")]
pub(super) use server::RawServerConfigFile;
