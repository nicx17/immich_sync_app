use serde::{Serialize, Deserialize};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

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
        }
    }
}

pub struct StateManager {
    state_file: PathBuf,
}

impl StateManager {
    pub fn new() -> Self {
        // Match Python: ~/.cache/mimick/status.json
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.cache"))
            .join("mimick");

        let state_file = cache_dir.join("status.json");
        Self { state_file }
    }

    pub fn write_state(&self, mut state: AppState) {
        state.timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
            
        if let Some(parent) = self.state_file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        
        if let Ok(content) = serde_json::to_string(&state) {
            let unique_ext = format!("tmp.{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos());
            let tmp_file = self.state_file.with_extension(unique_ext);
            if fs::write(&tmp_file, &content).is_ok() {
                if fs::rename(&tmp_file, &self.state_file).is_ok() {
                    log::debug!("State written: status={} progress={} processed={}/{}",
                        state.status, state.progress,
                        state.processed_count, state.total_queued);
                } else {
                    let _ = fs::remove_file(&tmp_file); // cleanup on fail
                    log::warn!("Failed to atomically rename state file");
                }
            } else {
                log::warn!("Failed to write temp state file");
            }
        }
    }

    pub fn read_state(&self) -> AppState {
        match fs::read_to_string(&self.state_file) {
            Ok(content) => {
                match serde_json::from_str(&content) {
                    Ok(state) => {
                        log::debug!("State read: status={}", { let s: &AppState = &state; s.status.as_str() }.to_string());
                        state
                    }
                    Err(e) => {
                        log::warn!("Failed to parse state file: {}", e);
                        AppState::default()
                    }
                }
            }
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
    }

    #[test]
    fn test_state_manager_write_read() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("status.json");
        
        // We override the state_file manually for testing
        let manager = StateManager { state_file: file_path.clone() };
        
        let mut state = AppState::default();
        state.status = "syncing".to_string();
        state.progress = 50;
        
        manager.write_state(state.clone());
        
        assert!(file_path.exists());
        
        let read_state = manager.read_state();
        assert_eq!(read_state.status, "syncing");
        assert_eq!(read_state.progress, 50);
    }
}
