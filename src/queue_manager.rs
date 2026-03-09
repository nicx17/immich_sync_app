use tokio::sync::{mpsc, Mutex};
use std::sync::Arc;
use std::path::PathBuf;
use std::fs;
use crate::api_client::ImmichApiClient;
use crate::state_manager::AppState;
use crate::notifications;

/// A file task with path, checksum, and optional album association (from per-path config).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileTask {
    pub path: String,
    pub checksum: String,
    /// Optional album ID (if already resolved)
    #[serde(default)]
    pub album_id: Option<String>,
    /// Album name to look up or create
    #[serde(default)]
    pub album_name: Option<String>,
}

pub struct QueueManager {
    sender: mpsc::Sender<FileTask>,
    /// Shared in-memory state — updated directly by workers, read by the UI.
    /// No disk I/O during uploads; disk is only written on graceful shutdown.
    shared_state: Arc<std::sync::Mutex<AppState>>,
    /// In-memory retry list — no per-failure disk writes.
    retry_list: Arc<std::sync::Mutex<Vec<FileTask>>>,
    retry_path: PathBuf,
}

impl QueueManager {
    pub fn new(
        api_client: Arc<ImmichApiClient>,
        workers: usize,
        shared_state: Arc<std::sync::Mutex<AppState>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<FileTask>(64);
        let rx = Arc::new(Mutex::new(rx));

        let retry_path = {
            let mut p = dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("~/.cache"))
                .join("mimick");
            p.push("retries.json");
            p
        };

        // Load persisted retries from disk and immediately clear the file.
        // Items are re-added only if they fail again this session.
        let loaded_retries = load_retries(&retry_path);
        if !loaded_retries.is_empty() {
            log::info!("Loaded {} item(s) from retry queue. Clearing file.", loaded_retries.len());
            let _ = fs::write(&retry_path, "[]");
            shared_state.lock().unwrap().failed_count = loaded_retries.len();
        }

        // In-memory retry list: no disk writes during a session.
        let retry_list = Arc::new(std::sync::Mutex::new(Vec::<FileTask>::new()));

        let qm = Self {
            sender: tx,
            shared_state: shared_state.clone(),
            retry_list: retry_list.clone(),
            retry_path: retry_path.clone(),
        };

        for i in 0..workers {
            let rx_clone = rx.clone();
            let tx_clone = qm.sender.clone();
            let api = api_client.clone();
            let state_ref = shared_state.clone();
            let retry_ref = retry_list.clone();

            tokio::spawn(async move {
                log::debug!("Worker {} started", i);
                loop {
                    let task = {
                        let mut receiver = rx_clone.lock().await;
                        receiver.recv().await
                    };

                    match task {
                        Some(file_task) => {
                            // Mark active; snapshot counters — single lock, no ordering risk.
                            let (pc, tq) = {
                                let mut s = state_ref.lock().unwrap();
                                s.active_workers += 1;
                                s.status = "uploading".to_string();
                                s.current_file = Some(file_task.path.clone());
                                s.queue_size = s.total_queued.saturating_sub(s.processed_count);
                                s.progress = if s.total_queued > 0 {
                                    ((s.processed_count as f32 / s.total_queued as f32) * 100.0) as u8
                                } else { 0 };
                                (s.processed_count, s.total_queued)
                            };

                            log::info!("Worker {} uploading [{}/{}]: {}", i, pc + 1, tq, file_task.path);

                            let t_start = std::time::Instant::now();
                            let success = handle_upload(&api, &file_task).await;
                            let elapsed = t_start.elapsed().as_secs_f32();

                            if success {
                                log::info!("Upload SUCCESS: {} ({:.2}s)", file_task.path, elapsed);

                                // Atomically drain the in-memory retry list (fast, no I/O).
                                let retries: Vec<FileTask> = {
                                    let mut rl = retry_ref.lock().unwrap();
                                    std::mem::take(&mut *rl)
                                };
                                if !retries.is_empty() {
                                    log::info!("Network active. Re-queuing {} retry item(s).", retries.len());
                                    {
                                        let mut s = state_ref.lock().unwrap();
                                        s.failed_count = s.failed_count.saturating_sub(retries.len());
                                        s.total_queued += retries.len();
                                    }
                                    // Release all locks before await
                                    for t in retries {
                                        let _ = tx_clone.send(t).await;
                                    }
                                }
                            } else {
                                log::warn!("Upload FAILED: {} ({:.2}s). Adding to retry queue.", file_task.path, elapsed);
                                // Push to in-memory list only — zero disk I/O per failure.
                                retry_ref.lock().unwrap().push(file_task);
                                let mut s = state_ref.lock().unwrap();
                                s.failed_count += 1;
                            }

                            // Update processed count and determine idle state.
                            let notify_msg = {
                                let mut s = state_ref.lock().unwrap();
                                s.processed_count += 1;
                                s.active_workers -= 1;
                                s.current_file = None;

                                if s.processed_count >= s.total_queued && s.active_workers == 0 {
                                    s.queue_size = 0;
                                    s.status = "idle".to_string();
                                    s.progress = 100;
                                    log::info!("All {} file(s) processed. Idle.", s.total_queued);
                                    Some(format!(
                                        "Processed {} file(s).",
                                        s.processed_count.saturating_sub(s.failed_count)
                                    ))
                                } else {
                                    s.queue_size = s.total_queued.saturating_sub(s.processed_count);
                                    s.progress = if s.total_queued > 0 {
                                        ((s.processed_count as f32 / s.total_queued as f32) * 100.0) as u8
                                    } else { 0 };
                                    s.status = "uploading".to_string();
                                    None
                                }
                            };

                            if let Some(msg) = notify_msg {
                                // Spawn in blocking thread so notify-send doesn't stall the worker.
                                let title = "Upload Complete".to_string();
                                tokio::task::spawn_blocking(move || {
                                    notifications::send(&title, &msg, Some(100));
                                });
                            }
                        }
                        None => {
                            log::debug!("Worker {} channel closed, exiting.", i);
                            break;
                        }
                    }
                }
            });
        }

        // Re-queue persisted retries after a short delay (let the daemon settle first).
        let sender_clone = qm.sender.clone();
        let state_ref2 = shared_state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            if !loaded_retries.is_empty() {
                {
                    let mut s = state_ref2.lock().unwrap();
                    // Retry items are now being actively queued — reset failed_count.
                    s.failed_count = 0;
                    s.total_queued += loaded_retries.len();
                }
                for task in loaded_retries {
                    log::info!("Re-queuing from retry: {}", task.path);
                    let _ = sender_clone.send(task).await;
                }
            }
        });

        qm
    }

    /// Add a file task to the upload queue.
    pub async fn add_to_queue(&self, task: FileTask) {
        log::debug!("Queuing: {}", task.path);
        {
            let mut s = self.shared_state.lock().unwrap();
            s.total_queued += 1;
            s.queue_size = s.total_queued.saturating_sub(s.processed_count);
        }
        if let Err(e) = self.sender.send(task).await {
            log::error!("Failed to send task to queue: {}", e);
        }
    }

    /// Persist any remaining in-memory retry items to disk (call on graceful shutdown).
    pub fn flush_retries(&self) {
        let retries = self.retry_list.lock().unwrap();
        if !retries.is_empty() {
            save_retries(&self.retry_path, &retries);
            log::info!("Flushed {} unfinished retry item(s) to disk.", retries.len());
        }
    }
}

/// Upload a file and add it to the appropriate album.
async fn handle_upload(api: &ImmichApiClient, task: &FileTask) -> bool {
    let asset_id = api.upload_asset(&task.path, &task.checksum).await;

    let asset_id = match asset_id {
        None => return false,
        Some(ref id) if id == "DUPLICATE" => {
            log::info!("Asset already on server: {}", task.path);
            return true;
        }
        Some(id) => id,
    };

    // Determine album name (fall back to parent directory name like Python does)
    let album_name = match (&task.album_name, &task.album_id) {
        (Some(name), _) if !name.is_empty() && name != "Default (Folder Name)" => name.clone(),
        _ => {
            std::path::Path::new(&task.path)
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Mimick".to_string())
        }
    };

    log::info!("Adding '{}' to album '{}'", task.path, album_name);

    let final_album_id = if let Some(ref id) = task.album_id {
        if !id.is_empty() {
            Some(id.clone())
        } else {
            api.get_or_create_album(&album_name).await
        }
    } else {
        api.get_or_create_album(&album_name).await
    };

    if let Some(album_id) = final_album_id {
        api.add_assets_to_album(&album_id, &[asset_id]).await;
    } else {
        log::warn!("Could not resolve album '{}'. Asset uploaded but not added to album.", album_name);
    }

    true
}

fn save_retries(path: &PathBuf, tasks: &[FileTask]) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(content) = serde_json::to_string(tasks) {
        let unique_ext = format!("tmp.{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        let tmp = path.with_extension(unique_ext);
        if fs::write(&tmp, content).is_ok() {
            if let Err(e) = fs::rename(&tmp, path) {
                let _ = fs::remove_file(&tmp);
                log::warn!("Failed to save retries: {}", e);
            }
        }
    }
}

fn load_retries(path: &PathBuf) -> Vec<FileTask> {
    if !path.exists() { return Vec::new(); }
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(e) => {
            log::error!("Failed to load retries: {}", e);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_filetask_serialization() {
        let task = FileTask {
            path: "/a/b.jpg".to_string(),
            checksum: "sha123".to_string(),
            album_id: Some("id1".to_string()),
            album_name: Some("Album".to_string()),
        };
        let js = serde_json::to_string(&task).unwrap();
        assert!(js.contains("sha123"));

        let deserialized: FileTask = serde_json::from_str(&js).unwrap();
        assert_eq!(deserialized.path, "/a/b.jpg");
        assert_eq!(deserialized.album_id.unwrap(), "id1");
    }

    #[test]
    fn test_retry_persistence() {
        let dir = tempdir().unwrap();
        let retry_path = dir.path().join("retries.json");

        let task = FileTask {
            path: "/a/1.jpg".to_string(),
            checksum: "hash1".to_string(),
            album_id: None,
            album_name: None,
        };

        let tasks = vec![task];
        save_retries(&retry_path, &tasks);
        let loaded = load_retries(&retry_path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, "/a/1.jpg");
    }
}
