//! Provides desktop notification helpers using the GIO notification portal.
//!
//! All functions are best-effort: if no running `gio::Application` is available,
//! the call is silently ignored. No notification is fired more than once per
//! concept per session -- callers are responsible for the guard logic.

use std::sync::atomic::{AtomicBool, Ordering};

/// Global kill-switch for desktop notifications, controlled by the user-facing
/// "Enable Notifications" toggle in Settings → Behavior.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Update the notifications enabled flag (called from config load and settings save).
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    log::debug!("Notifications enabled: {}", enabled);
}

/// Fired once when the upload queue drains after an active sync cycle.
///
/// `succeeded` and `failed` are the counts for that sync cycle.
pub fn send_sync_summary(succeeded: usize, failed: usize) {
    if succeeded == 0 && failed == 0 {
        return;
    }
    let title = "Sync complete".to_string();
    let processed = succeeded.saturating_add(failed);
    let body = if failed == 0 {
        format!("All {} file(s) processed. Idle.", processed)
    } else {
        format!(
            "All {} file(s) processed. Idle. {} failed and will be retried.",
            processed, failed
        )
    };
    send_gio(&title, &body, "sync-complete");
}

/// Fired once per session when consecutive uploads all fail due to connectivity issues.
pub fn send_connectivity_lost() {
    send_gio(
        "Mimick: Connection lost",
        "Could not reach the Immich server. Uploads will resume automatically when connectivity is restored.",
        "connectivity-lost",
    );
}

// ── Internal primitive ───────────────────────────────────────────────────────

/// Send a desktop notification via `gio::Notification`.
///
/// This uses the XDG notification portal under Flatpak and the native
/// desktop notification daemon on bare-metal installs. The themed icon
/// ensures the app icon renders correctly in both scenarios.
fn send_gio(title: &str, body: &str, notification_id: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        log::debug!("Notifications disabled; suppressing: {}", title);
        return;
    }

    let title = title.to_string();
    let body = body.to_string();
    let id = notification_id.to_string();

    glib::idle_add_once(move || {
        use gtk::prelude::ApplicationExt;

        let Some(app) = gtk::gio::Application::default() else {
            log::debug!("No default GIO application; skipping notification.");
            return;
        };

        let notification = gtk::gio::Notification::new(&title);
        notification.set_body(Some(&body));

        let icon = gtk::gio::ThemedIcon::new("io.github.nicx17.mimick");
        notification.set_icon(&icon);

        app.send_notification(Some(&id), &notification);
        log::debug!("Notification sent via GIO: {} - {}", title, body);
    });
}
