//! Consolidated application context passed through the UI and background services.
//!
//! Replaces the growing list of individual `Arc<T>` parameters that were previously
//! threaded through `build_settings_window()` and `open_settings_if_needed()`.

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

use crate::api_client::ImmichApiClient;
use crate::config::WatchPathEntry;
use crate::monitor::MonitorHandle;
use crate::queue_manager::QueueManager;
use crate::state_manager::AppState;

/// Shared application context holding all dependency handles that UI and background
/// tasks need. Wrapped in `Arc` at construction time so it can be cloned cheaply.
pub struct AppContext {
    pub shared_state: Arc<Mutex<AppState>>,
    pub api_client: Option<Arc<ImmichApiClient>>,
    pub queue_manager: Option<Arc<QueueManager>>,
    pub monitor_handle: Option<Arc<MonitorHandle>>,
    pub live_watch_paths: Option<Arc<Mutex<Vec<WatchPathEntry>>>>,
    pub sync_now_tx: Option<UnboundedSender<()>>,
}
