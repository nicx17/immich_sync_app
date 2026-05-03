//! Consolidated application context passed through the UI and background services.
//!
//! Replaces the growing list of individual `Arc<T>` parameters that were previously
//! threaded through `build_settings_window()` and `open_settings_if_needed()`.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

use crate::api_client::ImmichApiClient;
use crate::config::WatchPathEntry;
use crate::library::state::LibraryState;
use crate::library::thumbnail_cache::ThumbnailCache;
use crate::monitor::MonitorHandle;
use crate::queue_manager::QueueManager;
use crate::state_manager::AppState;
use crate::sync_index::SyncIndex;

/// Shared application context holding all dependency handles that UI and background
/// tasks need. Wrapped in `Arc` at construction time so it can be cloned cheaply.
pub struct AppContext {
    pub state: Arc<Mutex<AppState>>,
    pub api_client: Arc<ImmichApiClient>,
    pub queue_manager: Arc<QueueManager>,
    pub monitor_handle: Arc<MonitorHandle>,
    pub sync_index: Arc<Mutex<SyncIndex>>,
    pub live_watch_paths: Arc<Mutex<Vec<WatchPathEntry>>>,
    pub sync_now_tx: UnboundedSender<()>,
    pub thumbnail_cache: Arc<ThumbnailCache>,
    pub library_state: Arc<Mutex<LibraryState>>,
    pub library_timeline_active: AtomicBool,
    pub current_user_id: Arc<Mutex<Option<String>>>,
}
