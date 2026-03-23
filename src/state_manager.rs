//! Persistent status snapshots used to restore basic UI state across launches.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

/// Rolling queue/event status used by the settings window inspector.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QueueEvent {
    pub path: String,
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    pub timestamp: f64,
}

/// Status of an individual watch folder.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FolderSyncStatus {
    pub last_sync_at: Option<f64>,
    pub pending_count: usize,
    pub target_album: Option<String>,
    pub last_error: Option<String>,
}

/// Shared progress counters exposed to the settings window.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppState {
    pub queue_size: usize,
    pub total_queued: usize,
    pub processed_count: usize,
    #[serde(default)]
    pub failed_count: usize,
    /// In-flight worker count — not persisted to disk.
    #[serde(skip)]
    pub active_workers: usize,
    pub current_file: Option<String>,
    pub status: String,
    pub progress: u8,
    pub timestamp: f64,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub pause_reason: Option<String>,
    #[serde(default)]
    pub watched_folder_count: usize,
    #[serde(default)]
    pub active_server_route: Option<String>,
    #[serde(default)]
    pub last_successful_sync_at: Option<f64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_error_guidance: Option<String>,
    #[serde(default)]
    pub last_completed_file: Option<String>,
    #[serde(default)]
    pub diagnostics_exports: usize,
    #[serde(default)]
    pub recent_events: Vec<QueueEvent>,
    #[serde(default)]
    pub folder_statuses: std::collections::HashMap<String, FolderSyncStatus>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            queue_size: 0,
            total_queued: 0,
            processed_count: 0,
            failed_count: 0,
            active_workers: 0,
            current_file: None,
            status: "idle".to_string(),
            progress: 0,
            timestamp: 0.0,
            paused: false,
            pause_reason: None,
            watched_folder_count: 0,
            active_server_route: None,
            last_successful_sync_at: None,
            last_error: None,
            last_error_guidance: None,
            last_completed_file: None,
            diagnostics_exports: 0,
            recent_events: Vec::new(),
            folder_statuses: std::collections::HashMap::new(),
        }
    }
}

impl AppState {
    const MAX_EVENTS: usize = 80;

    pub fn record_event(
        &mut self,
        path: impl Into<String>,
        status: impl Into<String>,
        detail: Option<String>,
        attempts: u32,
    ) {
        let path = path.into();
        let status = status.into();
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        if let Some(existing) = self.recent_events.iter_mut().find(|evt| evt.path == path) {
            existing.status = status;
            existing.detail = detail;
            existing.attempts = attempts;
            existing.timestamp = timestamp;
        } else {
            self.recent_events.push(QueueEvent {
                path,
                status,
                detail,
                attempts,
                timestamp,
            });
        }

        self.recent_events
            .sort_by(|a, b| b.timestamp.total_cmp(&a.timestamp));
        self.recent_events.truncate(Self::MAX_EVENTS);
    }
}

pub struct StateManager {
    state_file: PathBuf,
}

impl StateManager {
    /// Point at the standard `status.json` cache path used by Mimick.
    pub fn new() -> Self {
        // Match Python: ~/.cache/mimick/status.json
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.cache"))
            .join("mimick");

        let state_file = cache_dir.join("status.json");
        Self { state_file }
    }

    /// Persist a status snapshot using a write-then-rename pattern.
    pub fn write_state(&self, mut state: AppState) {
        state.timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        if let Some(parent) = self.state_file.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Ok(content) = serde_json::to_string(&state) {
            let unique_ext = format!(
                "tmp.{}",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            let tmp_file = self.state_file.with_extension(unique_ext);
            if fs::write(&tmp_file, &content).is_ok() {
                if fs::rename(&tmp_file, &self.state_file).is_ok() {
                    log::debug!(
                        "State written: status={} progress={} processed={}/{}",
                        state.status,
                        state.progress,
                        state.processed_count,
                        state.total_queued
                    );
                } else {
                    let _ = fs::remove_file(&tmp_file); // cleanup on fail
                    log::warn!("Failed to atomically rename state file");
                }
            } else {
                log::warn!("Failed to write temp state file");
            }
        }
    }

    /// Load the last saved state or return defaults when no cache exists.
    pub fn read_state(&self) -> AppState {
        match fs::read_to_string(&self.state_file) {
            Ok(content) => match serde_json::from_str::<AppState>(&content) {
                Ok(state) => {
                    log::debug!("State read: status={}", state.status);
                    state
                }
                Err(e) => {
                    log::warn!("Failed to parse state file: {}", e);
                    AppState::default()
                }
            },
            Err(_) => AppState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.queue_size, 0);
        assert_eq!(state.status, "idle");
        assert_eq!(state.progress, 0);
        assert_eq!(state.watched_folder_count, 0);
        assert!(state.active_server_route.is_none());
        assert!(state.last_successful_sync_at.is_none());
        assert!(state.last_error_guidance.is_none());
    }

    #[test]
    fn test_state_manager_write_read() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("status.json");

        // We override the state_file manually for testing
        let manager = StateManager {
            state_file: file_path.clone(),
        };

        let state = AppState {
            status: "syncing".to_string(),
            progress: 50,
            ..AppState::default()
        };

        manager.write_state(state.clone());

        assert!(file_path.exists());

        let read_state = manager.read_state();
        assert_eq!(read_state.status, "syncing");
        assert_eq!(read_state.progress, 50);
    }

    #[test]
    fn test_state_manager_preserves_health_dashboard_fields() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("status.json");
        let manager = StateManager {
            state_file: file_path,
        };

        let state = AppState {
            watched_folder_count: 3,
            active_server_route: Some("LAN".into()),
            last_successful_sync_at: Some(1234.5),
            last_error: Some("Immich rejected the API key".into()),
            last_error_guidance: Some("Update the API key in Settings.".into()),
            ..AppState::default()
        };

        manager.write_state(state);
        let read_state = manager.read_state();

        assert_eq!(read_state.watched_folder_count, 3);
        assert_eq!(read_state.active_server_route.as_deref(), Some("LAN"));
        assert_eq!(read_state.last_successful_sync_at, Some(1234.5));
        assert_eq!(
            read_state.last_error.as_deref(),
            Some("Immich rejected the API key")
        );
        assert_eq!(
            read_state.last_error_guidance.as_deref(),
            Some("Update the API key in Settings.")
        );
    }

    #[test]
    fn test_record_event_updates_existing_entry() {
        let mut state = AppState::default();
        state.record_event("/tmp/a.jpg", "pending", Some("queued".into()), 1);
        state.record_event("/tmp/a.jpg", "failed", Some("retry".into()), 2);

        assert_eq!(state.recent_events.len(), 1);
        assert_eq!(state.recent_events[0].status, "failed");
        assert_eq!(state.recent_events[0].attempts, 2);
        assert_eq!(state.recent_events[0].detail.as_deref(), Some("retry"));
    }

    #[test]
    fn test_record_event_truncates_history() {
        let mut state = AppState::default();
        for i in 0..100 {
            state.record_event(format!("/tmp/{i}.jpg"), "pending", None, 1);
        }

        assert_eq!(state.recent_events.len(), 80);
    }
}
