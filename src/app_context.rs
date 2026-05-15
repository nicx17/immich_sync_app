//! Consolidated application context passed through the UI and background services.
//!
//! Replaces the growing list of individual `Arc<T>` parameters that were previously
//! threaded through `build_settings_window()` and `open_settings_if_needed()`.

use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

use crate::api_client::ImmichApiClient;
use crate::config::{Config, WatchPathEntry};
use crate::library::state::LibraryState;
use crate::library::thumbnail_cache::ThumbnailCache;
use crate::monitor::MonitorHandle;
use crate::queue_manager::QueueManager;
use crate::state_manager::AppState;
use crate::sync_index::ShardedSyncIndex;

/// TTL'd set of paths that Mimick itself just modified, used to suppress the
/// filesystem events those modifications cause. Used for two cases:
/// trashing (`expected_self_deletions`) and downloading from the album
/// (`expected_self_downloads`). Entries are one-shot — a real later event for
/// the same path still propagates normally.
#[derive(Default)]
pub struct RecentSelfPaths {
    inner: Mutex<HashMap<String, Instant>>,
}

impl RecentSelfPaths {
    const TTL: Duration = Duration::from_secs(60);

    pub fn mark(&self, path: &str) {
        let mut map = self.inner.lock();
        map.retain(|_, t| t.elapsed() < Self::TTL);
        map.insert(path.to_string(), Instant::now());
    }

    pub fn consume(&self, path: &str) -> bool {
        let mut map = self.inner.lock();
        map.retain(|_, t| t.elapsed() < Self::TTL);
        map.remove(path).is_some()
    }
}

/// Shared application context holding all dependency handles that UI and background
/// tasks need. Wrapped in `Arc` at construction time so it can be cloned cheaply.
pub struct AppContext {
    pub config: Arc<RwLock<Config>>,
    pub state: Arc<Mutex<AppState>>,
    pub api_client: Arc<ImmichApiClient>,
    pub queue_manager: Arc<QueueManager>,
    pub monitor_handle: Arc<MonitorHandle>,
    pub sync_index: Arc<ShardedSyncIndex>,
    pub live_watch_paths: Arc<Mutex<Vec<WatchPathEntry>>>,
    pub sync_now_tx: UnboundedSender<()>,
    pub thumbnail_cache: Arc<ThumbnailCache>,
    pub library_state: Arc<Mutex<LibraryState>>,
    pub library_timeline_active: AtomicBool,
    pub current_user_id: Arc<Mutex<Option<String>>>,
    pub expected_self_deletions: Arc<RecentSelfPaths>,
    pub expected_self_downloads: Arc<RecentSelfPaths>,
}
