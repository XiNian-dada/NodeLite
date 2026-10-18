//! Disposable ELF fixture: no NodeLite endpoints, production tokens or network access.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ROOT: &str = "/var/lib/nlup-lab";

#[derive(Deserialize)]
struct Identity {
    sequence: u64,
    mode: String,
}

#[derive(Serialize)]
struct Heartbeat<'a> {
    pid: u32,
    sequence: u64,
    digest: &'a str,
    config_digest: String,
    quiesced: bool,
    time: f64,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if fs::metadata("/proc/self")?.uid() == 0 {
        return Err("fixture must not run as root".into());
    }
    let executable = fs::read("/proc/self/exe")?;
    let marker = b"\nNLUP_FIXTURE=";
    let offset = executable
        .windows(marker.len())
        .rposition(|part| part == marker)
        .ok_or("missing fixture identity")?
        + marker.len();
    let identity: Identity = serde_json::from_slice(&executable[offset..])?;
    if std::env::args().any(|arg| arg == "--preflight") {
        if OpenOptions::new()
            .write(true)
            .open(format!("{ROOT}/agent/config"))
            .is_ok()
        {
            return Err("preflight sandbox permits config writes".into());
        }
        if identity.mode == "preflight_fail" {
            return Err("fixture preflight failure".into());
        }
        fs::read("/proc/stat")?;
        return Ok(());
    }
    if identity.mode == "crash" {
        std::process::exit(23);
    }
    let digest = format!("{:x}", Sha256::digest(&executable));
    loop {
        let config = fs::read(format!("{ROOT}/agent/config"))?;
        if identity.mode != "hang" {
            let heartbeat = Heartbeat {
                pid: std::process::id(),
                sequence: identity.sequence,
                digest: &digest,
                config_digest: format!("{:x}", Sha256::digest(config)),
                quiesced: Path::new(&format!("{ROOT}/code/quiesce")).exists(),
                time: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64(),
            };
            let bytes = serde_json::to_vec(&heartbeat)?;
            let tmp = format!("{ROOT}/status/heartbeat.tmp");
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o640)
                .open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(tmp, format!("{ROOT}/status/heartbeat.json"))?;
            fs::File::open(format!("{ROOT}/status"))?.sync_all()?;
        }
        thread::sleep(Duration::from_millis(150));
    }
}

fn main() {
    if run().is_err() {
        eprintln!("isolated fixture failed (details intentionally omitted)");
        std::process::exit(1);
    }
}
