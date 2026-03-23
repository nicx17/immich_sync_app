//! Desktop notification helpers.
//!
//! All functions are best-effort: if `notify-send` is not installed the call is
//! silently ignored.  No notification is fired more than once per concept per
//! session — callers are responsible for the guard logic.

use std::process::Command;

// ── Internal primitive ────────────────────────────────────────────────────────

fn send_raw(title: &str, message: &str) {
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name").arg("Mimick");
    cmd.arg(title);
    cmd.arg(message);

    match cmd.spawn() {
        Ok(mut child) => {
            let _ = child.wait();
            log::debug!("Notification sent: {} - {}", title, message);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // notify-send is optional.
        }
        Err(e) => log::error!("Failed to send notification: {}", e),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fired once when the upload queue drains after an active sync cycle.
///
/// `succeeded` and `failed` are the counts for that cycle.
pub fn send_sync_summary(succeeded: usize, failed: usize) {
    if succeeded == 0 && failed == 0 {
        return;
    }
    let title = if failed == 0 {
        "Sync complete".to_string()
    } else {
        format!("Sync complete ({} failed)", failed)
    };
    let body = if failed == 0 {
        format!("{} file(s) uploaded successfully.", succeeded)
    } else {
        format!(
            "{} file(s) uploaded. {} file(s) failed and will be retried.",
            succeeded, failed
        )
    };
    send_raw(&title, &body);
}

/// Fired once per session when consecutive uploads all fail due to connectivity.
pub fn send_connectivity_lost() {
    send_raw(
        "Mimick: Connection lost",
        "Could not reach the Immich server. Uploads will resume automatically when connectivity is restored.",
    );
}
