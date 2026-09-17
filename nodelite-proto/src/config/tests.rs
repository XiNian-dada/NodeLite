//! Configuration coverage runs for standalone Agent builds as well as the server feature.

mod agent;
#[cfg(feature = "server-config")]
mod section_defaults;
#[cfg(feature = "server-config")]
mod server;
#[cfg(feature = "server-config")]
mod server_defaults;
