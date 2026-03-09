use std::process::Command;

/// Sends desktop notifications via `notify-send`, matching the Python NotificationManager.
/// Silently ignored if notify-send is not installed.
pub fn send(title: &str, message: &str, progress: Option<u8>) {
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name").arg("Mimick");
    cmd.arg(title);
    cmd.arg(message);

    // Use synchronous hint so notifications replace each other (progress bar effect)
    cmd.arg("-h")
       .arg("string:x-canonical-private-synchronous:mimick-progress");

    if let Some(p) = progress {
        cmd.arg("-h").arg(format!("int:value:{}", p));
    }

    match cmd.spawn() {
        Ok(mut child) => {
            // Reap the child process to avoid zombies.
            // notify-send exits in < 50ms so this barely blocks.
            let _ = child.wait();
            log::debug!("Notification sent: {} - {}", title, message)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // notify-send not installed — silently ignore
        }
        Err(e) => log::error!("Failed to send notification: {}", e),
    }
}
