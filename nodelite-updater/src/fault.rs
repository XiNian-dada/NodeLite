//! Fault hooks are absent from normal builds and require a marked disposable VM.

pub(crate) fn checkpoint(_name: &str) -> crate::error::Result<()> {
    #[cfg(feature = "lab-faults")]
    {
        use crate::error::UpdateError;
        use std::path::Path;
        let marker = Path::new("/etc/nlup-lab-guest");
        if !std::fs::read_to_string(marker)
            .is_ok_and(|value| value.trim() == "isolated-prototype-v1")
        {
            return Err(UpdateError::Policy(
                "fault-enabled build requires disposable lab marker",
            ));
        }
        let path = Path::new("/etc/nodelite-updater/fault");
        if path.exists() && crate::policy::trusted_file(path, 128)?.as_slice() == _name.as_bytes() {
            // tmpfs signaling must not flush the transaction under investigation.
            let notice = serde_json::json!({"point": _name, "pid": std::process::id()});
            std::fs::write(
                "/run/nodelite-updater-lab/checkpoint.json",
                serde_json::to_vec(&notice)?,
            )?;
            // Stop only this isolated updater process until the harness kills it or QEMU.
            unsafe {
                libc::raise(libc::SIGSTOP);
            }
        }
    }
    Ok(())
}
