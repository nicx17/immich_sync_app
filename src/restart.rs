//! Small helpers for the "save and restart" flow.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Mark that the current process should relaunch Mimick after shutdown.
pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
}

/// Consume the restart flag so only one relaunch happens.
pub fn take_restart_request() -> bool {
    RESTART_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Spawn a replacement Mimick process, optionally reopening the settings window.
pub fn launch_replacement(open_settings: bool) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve the Mimick executable path: {err}"))?;

    let mut command = Command::new(executable);
    if open_settings {
        command.arg("--settings");
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Failed to restart Mimick: {err}"))
}

#[cfg(test)]
mod tests {
    use super::{request_restart, take_restart_request};

    #[test]
    fn test_restart_request_round_trip() {
        request_restart();
        assert!(take_restart_request());
        assert!(!take_restart_request());
    }
}
