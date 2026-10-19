//! Opt-in signed Agent updates; privileged service actions remain fixed systemd units.

#[cfg(target_os = "linux")]
mod activate;
#[cfg(target_os = "linux")]
mod defaults;
#[cfg(target_os = "linux")]
mod download;
#[cfg(target_os = "linux")]
mod enrollment;
mod error;
#[cfg(target_os = "linux")]
mod fault;
#[cfg(target_os = "linux")]
mod fs;
#[cfg(target_os = "linux")]
mod maintenance;
#[cfg(target_os = "linux")]
mod policy;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod state;
#[cfg(target_os = "linux")]
mod system;
#[cfg(target_os = "linux")]
mod transport;
#[cfg(target_os = "linux")]
mod trust;

pub use error::UpdateError;
#[cfg(target_os = "linux")]
pub use runtime::run;

#[cfg(not(target_os = "linux"))]
pub async fn run() -> Result<(), UpdateError> {
    Err(UpdateError::Policy(
        "the standalone updater requires Linux/systemd",
    ))
}
