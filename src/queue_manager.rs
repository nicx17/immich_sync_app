//! Upload queue orchestration, retry persistence, and sync-index updates.

use crate::api_client::ImmichApiClient;
use crate::notifications;
use crate::runtime_env;
use crate::state_manager::AppState;
use crate::sync_index::{SyncIndex, SyncTarget};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, mpsc};

/// A unit of work for the upload queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileTask {
    pub path: String,
    #[serde(default)]
    pub watch_path: String,
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

#[derive(Debug, Clone, Copy)]
pub struct EnvironmentPolicy {
    pub pause_on_metered_network: bool,
    pub pause_on_battery_power: bool,
}

impl QueueManager {
    pub fn new(
        api_client: Arc<ImmichApiClient>,
        workers: usize,
        shared_state: Arc<std::sync::Mutex<AppState>>,
        sync_index: Arc<std::sync::Mutex<SyncIndex>>,
        policy: EnvironmentPolicy,
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
                            wait_until_allowed(&state_ref, policy).await;

                            // Update the shared progress snapshot before handing off to the API.
                            let (pc, tq) = {
                                let mut s = state_ref.lock().unwrap();
                                s.active_workers += 1;
                                s.status = "uploading".to_string();
                                s.pause_reason = None;
                                s.current_file = Some(file_task.path.clone());
                                s.queue_size = s.total_queued.saturating_sub(s.processed_count);
                                s.progress = if s.total_queued > 0 {
                                    ((s.processed_count as f32 / s.total_queued as f32) * 100.0)
                                        as u8
                                } else {
                                    0
                                };
                                let attempts = current_attempt_count(&s, &file_task.path);
                                s.record_event(file_task.path.clone(), "uploading", None, attempts);
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
                            let active_route = api.active_route_label().await;
                            let latest_issue = api.latest_issue().await;

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

                                let mut s = state_ref.lock().unwrap();
                                let attempts = current_attempt_count(&s, &file_task.path);
                                s.active_server_route = active_route;
                                s.last_successful_sync_at = Some(unix_timestamp_now());
                                s.last_completed_file = Some(file_task.path.clone());
                                s.last_error = None;
                                s.last_error_guidance = None;
                                s.record_event(
                                    file_task.path.clone(),
                                    "completed",
                                    Some(format!("Finished in {:.2}s", elapsed)),
                                    attempts,
                                );
                                if let Some(target) = sync_target.as_ref() {
                                    let status = s
                                        .folder_statuses
                                        .entry(file_task.watch_path.clone())
                                        .or_default();
                                    status.pending_count = status.pending_count.saturating_sub(1);
                                    status.last_sync_at = Some(unix_timestamp_now());
                                    status.last_error = None;
                                    status.target_album = target.album_name.clone();
                                }
                            } else {
                                log::warn!(
                                    "Upload FAILED: {} ({:.2}s). Adding to retry queue.",
                                    file_task.path,
                                    elapsed
                                );
                                // Keep failed tasks in memory until the next graceful shutdown.
                                retry_ref.lock().unwrap().push(file_task.clone());
                                let mut s = state_ref.lock().unwrap();
                                s.failed_count += 1;
                                s.active_server_route = active_route;
                                let error_text = latest_issue
                                    .as_ref()
                                    .map(|issue| issue.summary.clone())
                                    .unwrap_or_else(|| {
                                        format!("Upload failed for {}", file_task.path)
                                    });
                                s.last_error = Some(error_text.clone());
                                s.last_error_guidance = latest_issue
                                    .as_ref()
                                    .map(|issue| issue.guidance.clone())
                                    .or_else(|| {
                                        Some(
                                            "Review the latest server and permission settings, then retry the failed item."
                                                .to_string(),
                                        )
                                    });

                                let status = s
                                    .folder_statuses
                                    .entry(file_task.watch_path.clone())
                                    .or_default();
                                status.pending_count = status.pending_count.saturating_sub(1);
                                status.last_error = Some(error_text);

                                let attempts = current_attempt_count(&s, &file_task.path);
                                s.record_event(
                                    file_task.path.clone(),
                                    "failed",
                                    Some("Queued for retry".to_string()),
                                    attempts,
                                );
                            }

                            // Update processed count and determine idle state.
                            let notify_msg = {
                                let mut s = state_ref.lock().unwrap();
                                s.processed_count += 1;
                                s.active_workers -= 1;
                                s.current_file = None;

                                if s.processed_count >= s.total_queued && s.active_workers == 0 {
                                    s.queue_size = 0;
                                    s.status = if s.paused {
                                        "paused".to_string()
                                    } else {
                                        "idle".to_string()
                                    };
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

            let status = s
                .folder_statuses
                .entry(task.watch_path.clone())
                .or_default();
            status.pending_count += 1;
            status.target_album = task.album_name.clone();

            let attempts = current_attempt_count(&s, &task.path);
            s.record_event(
                task.path.clone(),
                "pending",
                task.album_name
                    .clone()
                    .map(|name| format!("Target album: {}", name)),
                attempts,
            );
        }
        if let Err(e) = self.sender.send(task).await {
            log::error!("Failed to send task to queue: {}", e);
            self.pending_paths.lock().unwrap().remove(&e.0.path);

            // Revert the total_queued increment since it will never be processed.
            let mut s = self.shared_state.lock().unwrap();
            s.total_queued = s.total_queued.saturating_sub(1);
            s.queue_size = s.total_queued.saturating_sub(s.processed_count);
            let status = s.folder_statuses.entry(e.0.watch_path.clone()).or_default();
            status.pending_count = status.pending_count.saturating_sub(1);
            return false;
        }

        true
    }

    pub fn set_paused(&self, paused: bool, reason: Option<String>) {
        let mut state = self.shared_state.lock().unwrap();
        state.paused = paused;
        state.pause_reason = reason;
        state.status = if paused {
            "paused".to_string()
        } else if state.active_workers > 0 {
            "uploading".to_string()
        } else {
            "idle".to_string()
        };
    }

    pub fn is_paused(&self) -> bool {
        self.shared_state.lock().unwrap().paused
    }

    pub fn recent_events(&self) -> Vec<crate::state_manager::QueueEvent> {
        self.shared_state.lock().unwrap().recent_events.clone()
    }

    pub fn failed_tasks(&self) -> Vec<FileTask> {
        self.retry_list.lock().unwrap().clone()
    }

    pub fn clear_failed(&self) -> usize {
        let tasks = {
            let mut retries = self.retry_list.lock().unwrap();
            std::mem::take(&mut *retries)
        };
        if tasks.is_empty() {
            return 0;
        }

        {
            let mut pending = self.pending_paths.lock().unwrap();
            for task in &tasks {
                pending.remove(&task.path);
            }
        }

        let mut state = self.shared_state.lock().unwrap();
        state.failed_count = state.failed_count.saturating_sub(tasks.len());
        for task in &tasks {
            let attempts = current_attempt_count(&state, &task.path);
            state.record_event(
                task.path.clone(),
                "cleared",
                Some("Removed from retry queue".to_string()),
                attempts,
            );
        }

        tasks.len()
    }

    pub async fn retry_all_failed(&self) -> usize {
        let tasks = {
            let mut retries = self.retry_list.lock().unwrap();
            std::mem::take(&mut *retries)
        };
        self.requeue_failed(tasks, "Manual retry".to_string()).await
    }

    pub async fn retry_failed_path(&self, path: &str) -> bool {
        let task = {
            let mut retries = self.retry_list.lock().unwrap();
            let index = retries.iter().position(|task| task.path == path);
            index.map(|index| retries.remove(index))
        };

        if let Some(task) = task {
            self.requeue_failed(vec![task], "Manual retry".to_string())
                .await
                > 0
        } else {
            false
        }
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

    async fn requeue_failed(&self, tasks: Vec<FileTask>, detail: String) -> usize {
        if tasks.is_empty() {
            return 0;
        }

        {
            let mut state = self.shared_state.lock().unwrap();
            state.failed_count = state.failed_count.saturating_sub(tasks.len());
            state.total_queued += tasks.len();
            state.queue_size = state.total_queued.saturating_sub(state.processed_count);
            for task in &tasks {
                let attempts = current_attempt_count(&state, &task.path).saturating_add(1);
                state.record_event(task.path.clone(), "pending", Some(detail.clone()), attempts);
                let status = state
                    .folder_statuses
                    .entry(task.watch_path.clone())
                    .or_default();
                status.pending_count += 1;
            }
        }

        let mut queued = 0usize;
        for task in tasks {
            if self.sender.send(task).await.is_ok() {
                queued += 1;
            }
        }
        queued
    }
}

fn current_attempt_count(state: &AppState, path: &str) -> u32 {
    state
        .recent_events
        .iter()
        .find(|event| event.path == path)
        .map(|event| event.attempts)
        .unwrap_or(1)
}

fn unix_timestamp_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

async fn wait_until_allowed(
    state_ref: &Arc<std::sync::Mutex<AppState>>,
    policy: EnvironmentPolicy,
) {
    loop {
        let defer_reason = if state_ref.lock().unwrap().paused {
            state_ref
                .lock()
                .unwrap()
                .pause_reason
                .clone()
                .or_else(|| Some("Paused by user".to_string()))
        } else if policy.pause_on_metered_network && runtime_env::is_metered_connection() {
            Some("Deferred on metered network".to_string())
        } else if policy.pause_on_battery_power && runtime_env::is_on_battery_power() {
            Some("Deferred while on battery power".to_string())
        } else {
            None
        };

        if let Some(reason) = defer_reason {
            {
                let mut state = state_ref.lock().unwrap();
                state.status = "paused".to_string();
                state.pause_reason = Some(reason);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            continue;
        }

        let mut state = state_ref.lock().unwrap();
        if state.status == "paused" && !state.paused {
            state.status = "idle".to_string();
            state.pause_reason = None;
        }
        break;
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
    use crate::state_manager::AppState;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    type TestQueueManagerParts = (
        QueueManager,
        mpsc::Receiver<FileTask>,
        Arc<Mutex<AppState>>,
        Arc<Mutex<Vec<FileTask>>>,
        Arc<Mutex<HashSet<String>>>,
    );

    fn test_queue_manager(buffer: usize) -> TestQueueManagerParts {
        let (tx, rx) = mpsc::channel(buffer);
        let shared_state = Arc::new(Mutex::new(AppState::default()));
        let retry_list = Arc::new(Mutex::new(Vec::<FileTask>::new()));
        let pending_paths = Arc::new(Mutex::new(HashSet::<String>::new()));
        let retry_path = tempdir().unwrap().path().join("retries.json");

        (
            QueueManager {
                sender: tx,
                shared_state: shared_state.clone(),
                retry_list: retry_list.clone(),
                pending_paths: pending_paths.clone(),
                retry_path,
            },
            rx,
            shared_state,
            retry_list,
            pending_paths,
        )
    }

    #[test]
    fn test_filetask_serialization() {
        let task = FileTask {
            path: "/a/b.jpg".to_string(),
            watch_path: "/a".to_string(),
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
            watch_path: "/a".to_string(),
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

    #[tokio::test]
    async fn test_add_to_queue_rejects_duplicate_pending_path() {
        let (qm, mut rx, shared_state, _retry_list, pending_paths) = test_queue_manager(4);
        let task = FileTask {
            path: "/a/1.jpg".to_string(),
            watch_path: "/a".to_string(),
            checksum: "hash1".to_string(),
            album_id: None,
            album_name: Some("Album".into()),
            reassociate_only: false,
        };

        assert!(qm.add_to_queue(task.clone()).await);
        assert!(!qm.add_to_queue(task.clone()).await);

        let queued = rx.recv().await.unwrap();
        assert_eq!(queued.path, task.path);
        assert!(pending_paths.lock().unwrap().contains("/a/1.jpg"));
        assert_eq!(shared_state.lock().unwrap().total_queued, 1);
    }

    #[tokio::test]
    async fn test_retry_failed_path_requeues_and_updates_state() {
        let (qm, mut rx, shared_state, retry_list, pending_paths) = test_queue_manager(4);
        let task = FileTask {
            path: "/a/failed.jpg".to_string(),
            watch_path: "/a".to_string(),
            checksum: "hash1".to_string(),
            album_id: None,
            album_name: None,
            reassociate_only: false,
        };

        retry_list.lock().unwrap().push(task.clone());
        pending_paths.lock().unwrap().insert(task.path.clone());
        shared_state.lock().unwrap().failed_count = 1;

        assert!(qm.retry_failed_path(&task.path).await);

        let requeued = rx.recv().await.unwrap();
        assert_eq!(requeued.path, task.path);
        assert!(retry_list.lock().unwrap().is_empty());

        let state = shared_state.lock().unwrap();
        assert_eq!(state.failed_count, 0);
        assert_eq!(state.total_queued, 1);
        assert_eq!(state.recent_events[0].status, "pending");
        assert_eq!(state.recent_events[0].attempts, 2);
    }

    #[test]
    fn test_unix_timestamp_now_is_non_zero() {
        assert!(unix_timestamp_now() > 0.0);
    }

    #[test]
    fn test_clear_failed_removes_retry_entries_and_pending_paths() {
        let (qm, _rx, shared_state, retry_list, pending_paths) = test_queue_manager(4);
        let task = FileTask {
            path: "/a/failed.jpg".to_string(),
            watch_path: "/a".to_string(),
            checksum: "hash1".to_string(),
            album_id: None,
            album_name: None,
            reassociate_only: false,
        };

        retry_list.lock().unwrap().push(task.clone());
        pending_paths.lock().unwrap().insert(task.path.clone());
        shared_state.lock().unwrap().failed_count = 1;

        assert_eq!(qm.clear_failed(), 1);
        assert!(retry_list.lock().unwrap().is_empty());
        assert!(!pending_paths.lock().unwrap().contains(&task.path));

        let state = shared_state.lock().unwrap();
        assert_eq!(state.failed_count, 0);
        assert_eq!(state.recent_events[0].status, "cleared");
    }
}
