//! Thin wrapper around `notify-send` for desktop progress and completion notifications.

use std::process::Command;

/// Send a desktop notification if `notify-send` is available on the host.
pub fn send(title: &str, message: &str, progress: Option<u8>) {
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name").arg("Mimick");
    cmd.arg(title);
    cmd.arg(message);

    // Reuse a stable notification slot so progress updates replace each other.
    cmd.arg("-h")
        .arg("string:x-canonical-private-synchronous:mimick-progress");

    if let Some(p) = progress {
        cmd.arg("-h").arg(format!("int:value:{}", p));
    }

    match cmd.spawn() {
        Ok(mut child) => {
            // Reap the short-lived helper process so it does not become a zombie.
            let _ = child.wait();
            log::debug!("Notification sent: {} - {}", title, message)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // `notify-send` is optional, so missing support is not treated as an error.
        }
        Err(e) => log::error!("Failed to send notification: {}", e),
    }
}
