//! Opt-in diagnostics for code running inside Explorer's hosts. Logging is on
//! only while `%USERPROFILE%\AppData\LocalLow\PkgPeek` exists (LocalLow is
//! writable from the low-integrity preview host).

use std::io::Write;

pub fn log(message: impl AsRef<str>) {
    let Some(profile) = std::env::var_os("USERPROFILE") else { return };
    let dir = std::path::Path::new(&profile).join(r"AppData\LocalLow\PkgPeek");
    if !dir.is_dir() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("debug.log")) else {
        return;
    };
    let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let _ = writeln!(
        f,
        "{}.{:03} [{} pid {} tid {:?}] {}",
        now.as_secs() % 100_000,
        now.subsec_millis(),
        exe.unwrap_or_default(),
        std::process::id(),
        std::thread::current().id(),
        message.as_ref()
    );
}
