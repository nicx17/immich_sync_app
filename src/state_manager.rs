//! Stores persistent status snapshots to restore basic UI state across application launches.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

/// Represents a rolling queue/event status used by the settings window inspector.
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

/// Represents the status of an individual watch folder.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FolderSyncStatus {
    pub last_sync_at: Option<f64>,
    pub pending_count: usize,
    pub target_album: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub enum TransferDirection {
    #[default]
    Upload,
    Download,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TransferSnapshot {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub direction: TransferDirection,
    #[serde(default)]
    pub current_bytes: u64,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub session_total_bytes: u64,
    #[serde(default)]
    pub session_transferred_bytes: u64,
    #[serde(default)]
    pub completed_total_bytes: u64,
    #[serde(default)]
    pub completed_transferred_bytes: u64,
    #[serde(default)]
    pub instant_bps: f64,
    #[serde(default)]
    pub session_avg_bps: f64,
    #[serde(default)]
    pub session_started_at: Option<f64>,
    #[serde(default)]
    pub session_bytes_done: u64,
    #[serde(default)]
    pub active_item_label: Option<String>,
    #[serde(default)]
    pub active_route: Option<String>,
    #[serde(default)]
    pub last_upload_avg_bps: f64,
    #[serde(default)]
    pub last_download_avg_bps: f64,
    #[serde(skip)]
    pub last_tick_at: Option<f64>,
    #[serde(skip)]
    pub last_tick_bytes: u64,
    #[serde(skip)]
    pub active_uploads: usize,
    #[serde(skip)]
    pub active_downloads: usize,
    #[serde(skip)]
    pub active_item_bytes: HashMap<String, u64>,
    #[serde(skip)]
    pub active_item_totals: HashMap<String, u64>,
}

/// Contains shared progress counters exposed to the settings window.
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
    #[serde(default)]
    pub transfer: TransferSnapshot,
    #[serde(skip)]
    pub completed_upload_batches: u64,
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
            transfer: TransferSnapshot::default(),
            completed_upload_batches: 0,
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

    pub fn reset_runtime_state(&mut self) {
        self.transfer.reset_runtime();
    }
}

impl TransferSnapshot {
    pub fn reset_runtime(&mut self) {
        self.active = false;
        self.direction = TransferDirection::Upload;
        self.current_bytes = 0;
        self.total_bytes = None;
        self.session_total_bytes = 0;
        self.session_transferred_bytes = 0;
        self.completed_total_bytes = 0;
        self.completed_transferred_bytes = 0;
        self.instant_bps = 0.0;
        self.session_avg_bps = 0.0;
        self.session_started_at = None;
        self.session_bytes_done = 0;
        self.active_item_label = None;
        self.active_route = None;
        self.last_tick_at = None;
        self.last_tick_bytes = 0;
        self.active_uploads = 0;
        self.active_downloads = 0;
        self.active_item_bytes.clear();
        self.active_item_totals.clear();
    }

    fn should_switch_to(&self, direction: &TransferDirection) -> bool {
        if !self.active {
            return true;
        }
        match (&self.direction, direction) {
            (TransferDirection::Upload, TransferDirection::Download) => self.active_uploads == 0,
            (_, TransferDirection::Upload) => true,
            _ => self.direction == *direction,
        }
    }

    fn ensure_session(&mut self, direction: TransferDirection, route: Option<String>) {
        if self.should_switch_to(&direction) {
            self.active = true;
            self.direction = direction;
            self.current_bytes = 0;
            self.total_bytes = Some(0);
            self.session_total_bytes = 0;
            self.session_transferred_bytes = 0;
            self.completed_total_bytes = 0;
            self.completed_transferred_bytes = 0;
            self.instant_bps = 0.0;
            self.session_avg_bps = 0.0;
            self.session_started_at = None;
            self.session_bytes_done = 0;
            self.active_item_label = None;
            self.active_route = route;
            self.last_tick_at = None;
            self.last_tick_bytes = 0;
            self.active_item_bytes.clear();
            self.active_item_totals.clear();
        } else if route.is_some() {
            self.active_route = route;
        }
    }

    pub fn register_item(
        &mut self,
        direction: TransferDirection,
        item_id: impl Into<String>,
        total_bytes: Option<u64>,
        item_label: Option<String>,
        route: Option<String>,
    ) {
        self.ensure_session(direction.clone(), route.clone());
        let item_id = item_id.into();
        let previous_total = self
            .active_item_totals
            .insert(item_id.clone(), total_bytes.unwrap_or(0));
        if previous_total.is_none() {
            match direction {
                TransferDirection::Upload => self.active_uploads += 1,
                TransferDirection::Download => self.active_downloads += 1,
            }
        }
        self.active_item_bytes.entry(item_id).or_insert(0);
        let active_totals: u64 = self.active_item_totals.values().copied().sum();
        self.session_total_bytes = self.completed_total_bytes + active_totals;
        self.total_bytes = Some(self.session_total_bytes);
        if self.active_item_label.is_none() {
            self.active_item_label = item_label;
        }
    }

    pub fn begin_group(
        &mut self,
        direction: TransferDirection,
        item_label: Option<String>,
        route: Option<String>,
    ) {
        self.ensure_session(direction, route.clone());
        if self.active_item_label.is_none() {
            self.active_item_label = item_label;
        }
        if route.is_some() {
            self.active_route = route;
        }
    }

    pub fn update_item_total(&mut self, item_id: &str, total_bytes: u64) {
        self.active_item_totals
            .insert(item_id.to_string(), total_bytes);
        let active_totals: u64 = self.active_item_totals.values().copied().sum();
        self.session_total_bytes = self.completed_total_bytes + active_totals;
        self.total_bytes = Some(self.session_total_bytes);
    }

    pub fn update_item_bytes(
        &mut self,
        direction: TransferDirection,
        item_id: &str,
        bytes_done: u64,
        route: Option<String>,
    ) {
        if !self.active || self.direction != direction {
            return;
        }

        self.active_item_bytes
            .insert(item_id.to_string(), bytes_done);
        let active_bytes: u64 = self.active_item_bytes.values().copied().sum();
        self.session_transferred_bytes = self.completed_transferred_bytes + active_bytes;
        self.current_bytes = self.session_transferred_bytes;
        self.session_bytes_done = self.session_transferred_bytes;
        self.total_bytes = Some(self.session_total_bytes);

        let now = unix_timestamp_now();
        if self.session_started_at.is_none() {
            self.session_started_at = Some(now);
            self.last_tick_at = Some(now);
            self.last_tick_bytes = self.session_transferred_bytes;
            self.instant_bps = 0.0;
            self.session_avg_bps = 0.0;
            if route.is_some() {
                self.active_route = route;
            }
            return;
        }
        let mut do_update_speed = false;
        let mut previous_bytes = 0;

        let previous_tick = self.last_tick_at.unwrap_or(now);
        let elapsed = now - previous_tick;
        if elapsed >= 0.25 {
            do_update_speed = true;
            previous_bytes = self.last_tick_bytes;
        }

        if let Some(started) = self.session_started_at {
            self.session_avg_bps =
                self.session_transferred_bytes as f64 / (now - started).max(0.001);
        }

        if do_update_speed {
            let delta = self
                .session_transferred_bytes
                .saturating_sub(previous_bytes);
            self.instant_bps = delta as f64 / elapsed;
            self.last_tick_at = Some(now);
            self.last_tick_bytes = self.session_transferred_bytes;
        }

        if route.is_some() {
            self.active_route = route;
        }
    }

    pub fn finish_item(
        &mut self,
        direction: TransferDirection,
        item_id: &str,
        route: Option<String>,
    ) -> bool {
        let completed_avg = self.session_avg_bps;
        let item_final_bytes = self.active_item_bytes.remove(item_id).unwrap_or(0);
        let item_final_total = self.active_item_totals.remove(item_id).unwrap_or(0);

        self.completed_transferred_bytes += item_final_bytes;
        self.completed_total_bytes += item_final_total;

        let active_bytes: u64 = self.active_item_bytes.values().copied().sum();
        let active_total: u64 = self.active_item_totals.values().copied().sum();

        self.session_transferred_bytes = self.completed_transferred_bytes + active_bytes;
        self.session_total_bytes = self.completed_total_bytes + active_total;

        self.current_bytes = self.session_transferred_bytes;
        self.total_bytes = Some(self.session_total_bytes);

        match direction {
            TransferDirection::Upload => {
                self.active_uploads = self.active_uploads.saturating_sub(1)
            }
            TransferDirection::Download => {
                self.active_downloads = self.active_downloads.saturating_sub(1)
            }
        }

        match direction {
            TransferDirection::Upload => self.last_upload_avg_bps = completed_avg,
            TransferDirection::Download => self.last_download_avg_bps = completed_avg,
        }

        if route.is_some() {
            self.active_route = route;
        }

        if self.active_uploads == 0 && self.active_downloads == 0 {
            self.reset_runtime();
            return true;
        } else if self.active_uploads == 0 && self.direction == TransferDirection::Upload {
            self.direction = TransferDirection::Download;
            self.current_bytes = 0;
            self.total_bytes = Some(self.session_total_bytes);
            self.instant_bps = 0.0;
            self.session_avg_bps = 0.0;
            self.session_started_at = None;
            self.session_bytes_done = self.session_transferred_bytes;
            self.active_item_label = None;
            self.last_tick_at = None;
            self.last_tick_bytes = 0;
        }
        false
    }
}

fn unix_timestamp_now() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
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
                Ok(mut state) => {
                    state.reset_runtime_state();
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
        assert!(!state.transfer.active);
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
        assert!(!read_state.transfer.active);
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
        assert!(!read_state.transfer.active);
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

    #[test]
    fn test_transfer_snapshot_speed_updates() {
        let mut transfer = TransferSnapshot::default();
        transfer.register_item(
            TransferDirection::Upload,
            "file.jpg",
            Some(1_000),
            Some("file.jpg".into()),
            Some("LAN".into()),
        );
        let start = unix_timestamp_now() - 1.0;
        transfer.session_started_at = Some(start);
        transfer.last_tick_at = Some(start);
        transfer.last_tick_bytes = 0;
        transfer.update_item_bytes(
            TransferDirection::Upload,
            "file.jpg",
            512,
            Some("LAN".into()),
        );

        assert!(transfer.active);
        assert_eq!(transfer.current_bytes, 512);
        assert!(transfer.instant_bps > 0.0);
        assert!(transfer.session_avg_bps > 0.0);

        transfer.finish_item(TransferDirection::Upload, "file.jpg", Some("LAN".into()));
        assert!(!transfer.active);
    }
}
