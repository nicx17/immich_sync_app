//! Periodic remote-album reconciler.
//!
//! Re-runs the per-folder album↔folder diff on a fixed interval so changes
//! made directly in Immich (asset deletions, additions) propagate to local
//! folders without requiring an app restart.

use std::sync::Arc;
use std::time::Duration;

use crate::app_context::AppContext;
use crate::startup_scan::reconcile_entry;

const REMOTE_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub async fn run_album_reconciler(ctx: Arc<AppContext>) {
    loop {
        tokio::time::sleep(REMOTE_POLL_INTERVAL).await;

        if !ctx.config.read().data.background_sync_enabled {
            continue;
        }
        if ctx.queue_manager.is_paused() {
            continue;
        }
        if !ctx.api_client.check_connection().await {
            continue;
        }

        let watch_paths = ctx.config.read().data.watch_paths.clone();
        for entry in &watch_paths {
            reconcile_entry(ctx.clone(), entry).await;
        }
    }
}
