//! Upload queue orchestration, retry persistence, and sync-index updates.

use crate::api_client::ImmichApiClient;
use crate::notifications;
use crate::state_manager::AppState;
use crate::sync_index::{SyncIndex, SyncTarget};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

/// A unit of work for the upload queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileTask {
    pub path: String,
    pub checksum: String,
    /// Optional album ID if it has already been resolved.
    #[serde(default)]
    pub album_id: Option<String>,
    /// Album name to look up or create.
    #[serde(default)]
    pub album_name: Option<String>,
    /// True when the file already exists on the server and only album reassociation is needed.
    #[serde(default)]
    pub reassociate_only: bool,
}

pub struct QueueManager {
    sender: mpsc::Sender<FileTask>,
    /// Shared in-memory state that both workers and the UI read/update directly.
    shared_state: Arc<std::sync::Mutex<AppState>>,
    /// Failed tasks accumulated in memory and flushed on graceful shutdown.
    retry_list: Arc<std::sync::Mutex<Vec<FileTask>>>,
    /// Paths already queued or awaiting retry.
    ///
    /// This prevents duplicate entries when the startup scan and live watcher both
    /// notice the same file in a short time window.
    pending_paths: Arc<std::sync::Mutex<HashSet<String>>>,
    retry_path: PathBuf,
}

impl QueueManager {
    pub fn new(
        api_client: Arc<ImmichApiClient>,
        workers: usize,
        shared_state: Arc<std::sync::Mutex<AppState>>,
        sync_index: Arc<std::sync::Mutex<SyncIndex>>,
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

        // Load persisted retries and clear the file so only current-session failures are kept.
        let loaded_retries = load_retries(&retry_path);
        if !loaded_retries.is_empty() {
            log::info!(
                "Loaded {} item(s) from retry queue. Clearing file.",
                loaded_retries.len()
            );
            let _ = fs::write(&retry_path, "[]");
            shared_state.lock().unwrap().failed_count = loaded_retries.len();
        }

        // Retry state stays in memory during the session to avoid per-failure disk writes.
        let retry_list = Arc::new(std::sync::Mutex::new(Vec::<FileTask>::new()));
        let pending_paths = Arc::new(std::sync::Mutex::new(
            loaded_retries
                .iter()
                .map(|task| task.path.clone())
                .collect(),
        ));

        let qm = Self {
            sender: tx,
            shared_state: shared_state.clone(),
            retry_list: retry_list.clone(),
            pending_paths: pending_paths.clone(),
            retry_path: retry_path.clone(),
        };

        for i in 0..workers {
            let rx_clone = rx.clone();
            let tx_clone = qm.sender.clone();
            let api = api_client.clone();
            let state_ref = shared_state.clone();
            let retry_ref = retry_list.clone();
            let pending_ref = pending_paths.clone();
            let sync_index_ref = sync_index.clone();

            tokio::spawn(async move {
                log::debug!("Worker {} started", i);
                loop {
                    let task = {
                        let mut receiver = rx_clone.lock().await;
                        receiver.recv().await
                    };

                    match task {
                        Some(file_task) => {
                            // Update the shared progress snapshot before handing off to the API.
                            let (pc, tq) = {
                                let mut s = state_ref.lock().unwrap();
                                s.active_workers += 1;
                                s.status = "uploading".to_string();
                                s.current_file = Some(file_task.path.clone());
                                s.queue_size = s.total_queued.saturating_sub(s.processed_count);
                                s.progress = if s.total_queued > 0 {
                                    ((s.processed_count as f32 / s.total_queued as f32) * 100.0)
                                        as u8
                                } else {
                                    0
                                };
                                (s.processed_count, s.total_queued)
                            };

                            log::info!(
                                "Worker {} uploading [{}/{}]: {}",
                                i,
                                pc + 1,
                                tq,
                                file_task.path
                            );

                            let t_start = std::time::Instant::now();
                            let sync_target = handle_upload(&api, &file_task).await;
                            let success = sync_target.is_some();
                            let elapsed = t_start.elapsed().as_secs_f32();

                            if success {
                                log::info!("Upload SUCCESS: {} ({:.2}s)", file_task.path, elapsed);
                                pending_ref.lock().unwrap().remove(&file_task.path);

                                if let Some(target) = sync_target.as_ref()
                                    && let Err(err) = sync_index_ref.lock().unwrap().record_synced(
                                        &file_task.path,
                                        &file_task.checksum,
                                        target,
                                    )
                                {
                                    log::warn!(
                                        "Failed to update sync index for '{}': {}",
                                        file_task.path,
                                        err
                                    );
                                }

                                // Drain retries and requeue them once connectivity is working again.
                                let retries: Vec<FileTask> = {
                                    let mut rl = retry_ref.lock().unwrap();
                                    std::mem::take(&mut *rl)
                                };
                                if !retries.is_empty() {
                                    log::info!(
                                        "Network active. Re-queuing {} retry item(s).",
                                        retries.len()
                                    );
                                    {
                                        let mut s = state_ref.lock().unwrap();
                                        s.failed_count =
                                            s.failed_count.saturating_sub(retries.len());
                                        s.total_queued += retries.len();
                                    }
                                    // Release all locks before await
                                    for t in retries {
                                        let _ = tx_clone.send(t).await;
                                    }
                                }
                            } else {
                                log::warn!(
                                    "Upload FAILED: {} ({:.2}s). Adding to retry queue.",
                                    file_task.path,
                                    elapsed
                                );
                                // Keep failed tasks in memory until the next graceful shutdown.
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
                                        ((s.processed_count as f32 / s.total_queued as f32) * 100.0)
                                            as u8
                                    } else {
                                        0
                                    };
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

        // Re-queue persisted retries after startup so the main daemon can settle first.
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

    /// Add a file task to the upload queue and return whether it was accepted.
    pub async fn add_to_queue(&self, task: FileTask) -> bool {
        log::debug!("Queuing: {}", task.path);
        {
            let mut pending = self.pending_paths.lock().unwrap();
            if pending.contains(&task.path) {
                log::debug!("Skipping already pending task: {}", task.path);
                return false;
            }
            pending.insert(task.path.clone());
        }

        {
            let mut s = self.shared_state.lock().unwrap();
            s.total_queued += 1;
            s.queue_size = s.total_queued.saturating_sub(s.processed_count);
        }
        if let Err(e) = self.sender.send(task).await {
            log::error!("Failed to send task to queue: {}", e);
            self.pending_paths.lock().unwrap().remove(&e.0.path);
            return false;
        }

        true
    }

    /// Persist any in-memory retry items so they survive a clean shutdown.
    pub fn flush_retries(&self) {
        let retries = self.retry_list.lock().unwrap();
        if !retries.is_empty() {
            save_retries(&self.retry_path, &retries);
            log::info!(
                "Flushed {} unfinished retry item(s) to disk.",
                retries.len()
            );
        }
    }
}

/// Upload or reassociate a file, then ensure the resulting asset is present in the target album.
async fn handle_upload(api: &ImmichApiClient, task: &FileTask) -> Option<SyncTarget> {
    let asset_id = if task.reassociate_only {
        match api.find_existing_asset_id(&task.checksum).await {
            Some(existing) => Some(existing),
            None => api.upload_asset(&task.path, &task.checksum).await,
        }
    } else {
        api.upload_asset(&task.path, &task.checksum).await
    };

    let asset_id = match asset_id {
        None => return None,
        Some(ref id) if id == "DUPLICATE" => match api.find_existing_asset_id(&task.checksum).await
        {
            Some(existing) => existing,
            None => {
                log::info!("Asset already on server: {}", task.path);
                return Some(SyncTarget {
                    album_name: task
                        .album_name
                        .clone()
                        .or_else(|| infer_album_name(&task.path)),
                    album_id: task.album_id.clone(),
                });
            }
        },
        Some(id) => id,
    };

    // Fall back to the parent directory name when no explicit album name is configured.
    let album_name = match (&task.album_name, &task.album_id) {
        (Some(name), _) if !name.is_empty() && name != "Default (Folder Name)" => name.clone(),
        _ => std::path::Path::new(&task.path)
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Mimick".to_string()),
    };

    log::info!("Adding '{}' to album '{}'", task.path, album_name);

    let mut final_album_id = if let Some(ref id) = task.album_id {
        if !id.is_empty() {
            Some(id.clone())
        } else {
            api.get_or_create_album(&album_name).await
        }
    } else {
        api.get_or_create_album(&album_name).await
    };

    if let Some(album_id) = final_album_id.clone() {
        if !api
            .add_assets_to_album(&album_id, std::slice::from_ref(&asset_id))
            .await
        {
            if let Some(album_name) = task
                .album_name
                .clone()
                .or_else(|| infer_album_name(&task.path))
            {
                log::warn!(
                    "Album '{}' may be stale or deleted. Refreshing album resolution.",
                    album_id
                );
                final_album_id = api.resolve_album_by_name(&album_name, true).await;
                if let Some(ref refreshed_id) = final_album_id {
                    if !api
                        .add_assets_to_album(refreshed_id, std::slice::from_ref(&asset_id))
                        .await
                    {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
    } else {
        log::warn!(
            "Could not resolve album '{}'. Asset uploaded but not added to album.",
            album_name
        );
        return None;
    }

    Some(SyncTarget {
        album_name: Some(album_name),
        album_id: final_album_id,
    })
}

fn infer_album_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
}

fn save_retries(path: &PathBuf, tasks: &[FileTask]) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(content) = serde_json::to_string(tasks) {
        let unique_ext = format!(
            "tmp.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let tmp = path.with_extension(unique_ext);
        if fs::write(&tmp, content).is_ok()
            && let Err(e) = fs::rename(&tmp, path)
        {
            let _ = fs::remove_file(&tmp);
            log::warn!("Failed to save retries: {}", e);
        }
    }
}

fn load_retries(path: &PathBuf) -> Vec<FileTask> {
    if !path.exists() {
        return Vec::new();
    }
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
            reassociate_only: false,
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
            reassociate_only: false,
        };

        let tasks = vec![task];
        save_retries(&retry_path, &tasks);
        let loaded = load_retries(&retry_path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, "/a/1.jpg");
    }
}
