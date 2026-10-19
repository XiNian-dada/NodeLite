//! Opt-in signed Agent updates; privileged service actions remain fixed systemd units.

mod activate;
mod defaults;
mod download;
mod error;
mod fault;
mod fs;
mod policy;
mod runtime;
mod state;
mod system;
mod transport;

pub use error::UpdateError;
pub use runtime::run;
