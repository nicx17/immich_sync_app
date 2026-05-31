//! Manages upload queue orchestration, retry persistence, and sync-index updates.
//!
//! A configurable pool of async workers drains the upload queue in parallel.
//! Failed tasks are moved to a retry queue with exponential back-off. The
//! queue pauses automatically when environment policies (metered network,
//! battery power, quiet hours) are active, and resumes when conditions clear.

use crate::api_client::{ImmichApiClient, TransferProgressCallback};
use crate::notifications;
use crate::runtime_env;
use crate::state_manager::{AppState, TransferDirection};
use crate::sync_index::{ShardedSyncIndex, SyncTarget};
use chrono::Timelike;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

/// Helper to generate progress callback callbacks that update global transfer metrics.
fn upload_progress_callback(
    shared_state: &Arc<parking_lot::Mutex<AppState>>,
    item_id: String,
) -> TransferProgressCallback {
    let state_ref = shared_state.clone();
    Arc::new(move |bytes_done, total_bytes| {
        let mut state = state_ref.lock();
        let route = state.active_server_route.clone();
        if let Some(total_bytes) = total_bytes {
            let current = state
                .transfer
                .active_item_totals
                .get(&item_id)
                .copied()
                .unwrap_or(0);
            if current == 0 {
                state.transfer.update_item_total(&item_id, total_bytes);
            }
        }
        state
            .transfer
            .update_item_bytes(TransferDirection::Upload, &item_id, bytes_done, route);
    })
}

/// Represents a unit of work for the upload queue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileTask {
    /// Absolute local file path.
    pub path: String,
    /// Absolute watch path prefix under which the file resides.
    #[serde(default)]
    pub watch_path: String,
    /// Precomputed file SHA-1 checksum.
    pub checksum: String,
    /// Target Immich album identifier if already resolved.
    #[serde(default)]
    pub album_id: Option<String>,
    /// Mapped Immich album name.
    #[serde(default)]
    pub album_name: Option<String>,
    /// True if the asset is already on the server and only needs album addition.
    #[serde(default)]
    pub reassociate_only: bool,
    /// Manual / ad-hoc uploads that must land in the library without being
    /// pinned to any album. Watch-folder syncs leave this false so the
    /// existing parent-dir-as-album fallback still applies.
    #[serde(default)]
    pub skip_album: bool,
    /// Absolute path to an XMP sidecar file to attach during upload.
    /// `None` when no companion sidecar exists or sidecar upload is disabled.
    #[serde(default)]
    pub sidecar_path: Option<String>,
}

pub struct QueueManager {
    /// Internal task sender channel.
    sender: mpsc::Sender<FileTask>,
    /// Shared in-memory state accessed by both workers and the UI.
    shared_state: Arc<parking_lot::Mutex<AppState>>,
    /// Failed tasks accumulated in memory and flushed during graceful shutdown.
    retry_list: Arc<parking_lot::Mutex<Vec<FileTask>>>,
    /// Paths already queued or awaiting retry to avoid duplication.
    pending_paths: Arc<parking_lot::Mutex<HashSet<String>>>,
    /// Path of persistent retries file.
    retry_path: PathBuf,
    /// Shared current transmission limits and environmental constraints.
    policy: Arc<parking_lot::Mutex<EnvironmentPolicy>>,
    /// Active worker concurrency limit wrapper.
    worker_limit: Arc<AtomicUsize>,
    /// Sync summary notification batch states tracker.
    batch_notify_state: Arc<parking_lot::Mutex<BatchNotifyState>>,
    /// Flag indicating whether new tasks are accepted.
    accepting_new: Arc<AtomicBool>,
    /// Internal shutdown cancellation coordinator.
    shutdown_token: CancellationToken,
}

#[derive(Debug, Default)]
struct BatchNotifyState {
    /// Active status indicating whether a batch notifications window is open.
    active: bool,
    /// True if a notification summary sweep is already planned.
    notify_scheduled: bool,
    /// Unique identifier of the current batch.
    current_batch_id: u64,
    /// Last unique identifier that triggered desktop notification.
    last_notified_batch_id: u64,
    /// Processed count recorded at beginning of batch.
    start_processed: usize,
    /// Failed count recorded at beginning of batch.
    start_failed: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct EnvironmentPolicy {
    /// True if syncing should pause when metered connections are detected.
    pub pause_on_metered_network: bool,
    /// True if syncing should pause when battery power is detected.
    pub pause_on_battery_power: bool,
    /// First local clock hour (0–23) of the quiet window. `None` = disabled.
    pub quiet_hours_start: Option<u8>,
    /// Last local clock hour (0–23, exclusive) of the quiet window.
    pub quiet_hours_end: Option<u8>,
}

impl QueueManager {
    /// Initialize a new QueueManager and kick off worker threads.
    pub fn new(
        api_client: Arc<ImmichApiClient>,
        workers: usize,
        shared_state: Arc<parking_lot::Mutex<AppState>>,
        sync_index: Arc<ShardedSyncIndex>,
        policy: EnvironmentPolicy,
    ) -> Self {
        const MAX_WORKERS: usize = 10;
        let (tx, rx) = mpsc::channel::<FileTask>(64);
        let rx = Arc::new(Mutex::new(rx));

        let retry_path = {
            let mut p = crate::profile::cache_dir()
                .unwrap_or_else(|| PathBuf::from("~/.cache").join(crate::profile::dir_segment()));
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
            shared_state.lock().failed_count = loaded_retries.len();
        }

        // Retry state stays in memory during the session to avoid per-failure disk writes.
        let retry_list = Arc::new(parking_lot::Mutex::new(Vec::<FileTask>::new()));
        let pending_paths = Arc::new(parking_lot::Mutex::new(
            loaded_retries
                .iter()
                .map(|task| task.path.clone())
                .collect(),
        ));
        let policy_ref = Arc::new(parking_lot::Mutex::new(policy));
        let worker_limit = Arc::new(AtomicUsize::new(workers.clamp(1, MAX_WORKERS)));
        let batch_notify_state = Arc::new(parking_lot::Mutex::new(BatchNotifyState::default()));
        let connectivity_lost_notified = Arc::new(parking_lot::Mutex::new(false));
        let consecutive_failures = Arc::new(parking_lot::Mutex::new(0usize));
        let accepting_new = Arc::new(AtomicBool::new(true));
        let shutdown_token = CancellationToken::new();

        let qm = Self {
            sender: tx,
            shared_state: shared_state.clone(),
            retry_list: retry_list.clone(),
            pending_paths: pending_paths.clone(),
            retry_path: retry_path.clone(),
            policy: policy_ref.clone(),
            worker_limit: worker_limit.clone(),
            batch_notify_state: batch_notify_state.clone(),
            accepting_new: accepting_new.clone(),
            shutdown_token: shutdown_token.clone(),
        };

        for i in 0..MAX_WORKERS {
            let rx_clone = rx.clone();
            let tx_clone = qm.sender.clone();
            let api = api_client.clone();
            let state_ref = shared_state.clone();
            let retry_ref = retry_list.clone();
            let pending_ref = pending_paths.clone();
            let sync_index_ref = sync_index.clone();
            let batch_notify_ref = batch_notify_state.clone();
            let connectivity_notified_ref = connectivity_lost_notified.clone();
            let consec_fail_ref = consecutive_failures.clone();
            let policy_ref = policy_ref.clone();
            let worker_limit_ref = worker_limit.clone();
            let cancel = shutdown_token.clone();

            tokio::spawn(async move {
                log::debug!("Worker {} started", i);
                loop {
                    while i >= worker_limit_ref.load(Ordering::Relaxed) {
                        if cancel.is_cancelled() {
                            log::debug!("Worker {} cancelled while throttled, exiting.", i);
                            return;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                    }

                    let task = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => None,
                        msg = async {
                            let mut receiver = rx_clone.lock().await;
                            receiver.recv().await
                        } => msg,
                    };

                    match task {
                        Some(file_task) => {
                            wait_until_allowed(&state_ref, &policy_ref).await;

                            // Update the shared progress snapshot before handing off to the API.
                            let (pc, tq) = {
                                let mut s = state_ref.lock();
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
                                let item_label = std::path::Path::new(&file_task.path)
                                    .file_name()
                                    .map(|name| name.to_string_lossy().to_string())
                                    .or_else(|| Some(file_task.path.clone()));
                                let route = s.active_server_route.clone();
                                s.transfer.register_item(
                                    TransferDirection::Upload,
                                    file_task.path.clone(),
                                    std::fs::metadata(&file_task.path)
                                        .ok()
                                        .map(|meta| meta.len()),
                                    item_label,
                                    route,
                                );
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
                            let sync_target = tokio::select! {
                                biased;
                                _ = cancel.cancelled() => {
                                    log::warn!(
                                        "Upload cancelled by shutdown: {}",
                                        file_task.path
                                    );
                                    None
                                }
                                res = handle_upload(
                                    &api,
                                    &file_task,
                                    upload_progress_callback(&state_ref, file_task.path.clone()),
                                ) => res,
                            };
                            let success = sync_target.is_some();
                            let elapsed = t_start.elapsed().as_secs_f32();
                            let active_route = api.active_route_label().await;
                            let latest_issue = api.latest_issue().await;

                            if success {
                                log::info!("Upload SUCCESS: {} ({:.2}s)", file_task.path, elapsed);
                                pending_ref.lock().remove(&file_task.path);

                                if let Some(target) = sync_target.as_ref()
                                    && let Err(err) = sync_index_ref.record_synced(
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
                                    let mut rl = retry_ref.lock();
                                    std::mem::take(&mut *rl)
                                };
                                if !retries.is_empty() {
                                    log::info!(
                                        "Network active. Re-queuing {} retry item(s).",
                                        retries.len()
                                    );
                                    {
                                        let mut s = state_ref.lock();
                                        let mut batch_state = batch_notify_ref.lock();
                                        activate_batch_if_needed(&mut batch_state, &s);
                                        s.failed_count =
                                            s.failed_count.saturating_sub(retries.len());
                                        s.total_queued += retries.len();
                                    }
                                    // Release all locks before await
                                    for t in retries {
                                        let _ = tx_clone.send(t).await;
                                    }
                                }

                                record_upload_success(
                                    &state_ref,
                                    &file_task,
                                    sync_target.as_ref(),
                                    active_route,
                                    elapsed,
                                );
                            } else {
                                record_upload_failure(
                                    &state_ref,
                                    &retry_ref,
                                    &file_task,
                                    active_route,
                                    latest_issue.as_ref(),
                                    elapsed,
                                );
                            }

                            // Track consecutive failures for connectivity-lost detection.
                            track_consecutive_failures(
                                success,
                                &consec_fail_ref,
                                &connectivity_notified_ref,
                            );

                            // Update processed count and determine idle state.
                            let summary_batch = finalize_upload_progress(
                                &state_ref,
                                &batch_notify_ref,
                                &sync_index_ref,
                                &file_task.path,
                                success,
                            );

                            if let Some(batch_id) = summary_batch {
                                schedule_batch_notification(
                                    state_ref.clone(),
                                    batch_notify_ref.clone(),
                                    batch_id,
                                );
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
                    let mut s = state_ref2.lock();
                    let mut batch_state = batch_notify_state.lock();
                    activate_batch_if_needed(&mut batch_state, &s);
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
        if !self.accepting_new.load(Ordering::Relaxed) {
            log::debug!("Refusing new task during shutdown: {}", task.path);
            return false;
        }
        log::debug!("Queuing: {}", task.path);
        {
            let mut pending = self.pending_paths.lock();
            if pending.contains(&task.path) {
                log::debug!("Skipping already pending task: {}", task.path);
                return false;
            }
            pending.insert(task.path.clone());
        }

        {
            let mut s = self.shared_state.lock();
            let mut batch_state = self.batch_notify_state.lock();
            activate_batch_if_needed(&mut batch_state, &s);
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
            self.pending_paths.lock().remove(&e.0.path);

            // Revert the total_queued increment since it will never be processed.
            let mut s = self.shared_state.lock();
            s.total_queued = s.total_queued.saturating_sub(1);
            s.queue_size = s.total_queued.saturating_sub(s.processed_count);
            let route = s.active_server_route.clone();
            let completed_batch =
                s.transfer
                    .finish_item(TransferDirection::Upload, &e.0.path, route);
            if completed_batch {
                s.completed_upload_batches = s.completed_upload_batches.saturating_add(1);
            }
            let status = s.folder_statuses.entry(e.0.watch_path.clone()).or_default();
            status.pending_count = status.pending_count.saturating_sub(1);
            return false;
        }

        true
    }

    pub fn set_paused(&self, paused: bool, reason: Option<String>) {
        let mut state = self.shared_state.lock();
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
        self.shared_state.lock().paused
    }

    pub fn set_worker_limit(&self, workers: u8) {
        self.worker_limit
            .store((workers as usize).clamp(1, 10), Ordering::Relaxed);
    }

    pub fn update_environment_policy(&self, policy: EnvironmentPolicy) {
        *self.policy.lock() = policy;
    }

    pub fn recent_events(&self) -> Vec<crate::state_manager::QueueEvent> {
        self.shared_state.lock().recent_events.clone()
    }

    pub fn failed_tasks(&self) -> Vec<FileTask> {
        self.retry_list.lock().clone()
    }

    pub fn clear_failed(&self) -> usize {
        let tasks = {
            let mut retries = self.retry_list.lock();
            std::mem::take(&mut *retries)
        };
        if tasks.is_empty() {
            return 0;
        }

        {
            let mut pending = self.pending_paths.lock();
            for task in &tasks {
                pending.remove(&task.path);
            }
        }

        let mut state = self.shared_state.lock();
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
            let mut retries = self.retry_list.lock();
            std::mem::take(&mut *retries)
        };
        self.requeue_failed(tasks, "Manual retry".to_string()).await
    }

    pub async fn retry_failed_path(&self, path: &str) -> bool {
        let task = {
            let mut retries = self.retry_list.lock();
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

    /// Stop accepting new tasks, wait up to `deadline` for active uploads
    /// to finish, then hard-cancel anything still in flight.
    ///
    /// In-flight uploads that get cancelled at the deadline land in the retry
    /// list (treated as failures) so `flush_retries` will persist them for the
    /// next session.
    pub async fn shutdown(&self, deadline: Duration) {
        self.accepting_new.store(false, Ordering::Relaxed);
        let active_now = self.shared_state.lock().active_workers;
        if active_now == 0 {
            self.shutdown_token.cancel();
            return;
        }
        log::info!(
            "Draining {} active upload(s); waiting up to {:?}...",
            active_now,
            deadline
        );

        let start = Instant::now();
        while start.elapsed() < deadline {
            if self.shared_state.lock().active_workers == 0 {
                log::info!("All active uploads finished within deadline.");
                self.shutdown_token.cancel();
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let stuck = self.shared_state.lock().active_workers;
        log::warn!(
            "Shutdown deadline exceeded; cancelling {} in-flight upload(s).",
            stuck
        );
        self.shutdown_token.cancel();
        // Brief grace so cancelled futures can record themselves as failed before flush.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    /// Persist any in-memory retry items so they survive a clean shutdown.
    pub fn flush_retries(&self) {
        let retries = self.retry_list.lock();
        if !retries.is_empty() {
            save_retries(&self.retry_path, &retries);
            log::info!(
                "Flushed {} unfinished retry item(s) to disk.",
                retries.len()
            );
        }
    }

    /// Requeue failed task lists with specified details string.
    async fn requeue_failed(&self, tasks: Vec<FileTask>, detail: String) -> usize {
        if tasks.is_empty() {
            return 0;
        }

        {
            let mut state = self.shared_state.lock();
            let mut batch_state = self.batch_notify_state.lock();
            activate_batch_if_needed(&mut batch_state, &state);
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

/// Get current sync attempt count for a path based on recent events history.
fn current_attempt_count(state: &AppState, path: &str) -> u32 {
    state
        .recent_events
        .iter()
        .find(|event| event.path == path)
        .map(|event| event.attempts)
        .unwrap_or(1)
}

/// Activate a batch tracking window if none is currently active.
/// Record state updates after a successful upload.
fn record_upload_success(
    state_ref: &Arc<parking_lot::Mutex<AppState>>,
    task: &FileTask,
    sync_target: Option<&SyncTarget>,
    active_route: Option<String>,
    elapsed: f32,
) {
    let mut s = state_ref.lock();
    let attempts = current_attempt_count(&s, &task.path);
    s.active_server_route = active_route;
    s.last_successful_sync_at = Some(unix_timestamp_now());
    s.last_completed_file = Some(task.path.clone());
    s.last_error = None;
    s.last_error_guidance = None;
    s.record_event(
        task.path.clone(),
        "completed",
        Some(format!("Finished in {:.2}s", elapsed)),
        attempts,
    );
    if let Some(target) = sync_target {
        let status = s
            .folder_statuses
            .entry(task.watch_path.clone())
            .or_default();
        status.pending_count = status.pending_count.saturating_sub(1);
        status.last_sync_at = Some(unix_timestamp_now());
        status.last_error = None;
        status.target_album = target.album_name.clone();
    }
}

/// Record state updates after a failed upload.
fn record_upload_failure(
    state_ref: &Arc<parking_lot::Mutex<AppState>>,
    retry_ref: &Arc<parking_lot::Mutex<Vec<FileTask>>>,
    task: &FileTask,
    active_route: Option<String>,
    latest_issue: Option<&crate::api_client::ApiIssue>,
    elapsed: f32,
) {
    log::warn!(
        "Upload FAILED: {} ({:.2}s). Adding to retry queue.",
        task.path,
        elapsed
    );
    retry_ref.lock().push(task.clone());
    let mut s = state_ref.lock();
    s.failed_count += 1;
    s.active_server_route = active_route;
    let error_text = latest_issue
        .map(|issue| issue.summary.clone())
        .unwrap_or_else(|| format!("Upload failed for {}", task.path));
    s.last_error = Some(error_text.clone());
    s.last_error_guidance = latest_issue
        .map(|issue| issue.guidance.clone())
        .or_else(|| {
            Some(
                "Review the latest server and permission settings, then retry the failed item."
                    .to_string(),
            )
        });
    let status = s
        .folder_statuses
        .entry(task.watch_path.clone())
        .or_default();
    status.pending_count = status.pending_count.saturating_sub(1);
    status.last_error = Some(error_text);
    let attempts = current_attempt_count(&s, &task.path);
    s.record_event(
        task.path.clone(),
        "failed",
        Some("Queued for retry".to_string()),
        attempts,
    );
}

/// Track consecutive failures and fire a connectivity-lost notification after 3+.
fn track_consecutive_failures(
    success: bool,
    consec_fail_ref: &Arc<parking_lot::Mutex<usize>>,
    connectivity_notified_ref: &Arc<parking_lot::Mutex<bool>>,
) {
    if success {
        *consec_fail_ref.lock() = 0;
        return;
    }
    let mut cf = consec_fail_ref.lock();
    *cf += 1;
    if *cf >= 3 {
        let mut notified = connectivity_notified_ref.lock();
        if !*notified {
            *notified = true;
            notifications::send_connectivity_lost();
        }
    }
}

/// Finalize upload progress: update counters, detect queue idle, return batch ID if
/// a summary notification should be scheduled.
fn finalize_upload_progress(
    state_ref: &Arc<parking_lot::Mutex<AppState>>,
    batch_notify_ref: &Arc<parking_lot::Mutex<BatchNotifyState>>,
    sync_index_ref: &Arc<ShardedSyncIndex>,
    path: &str,
    success: bool,
) -> Option<u64> {
    let mut s = state_ref.lock();
    if success {
        s.processed_count += 1;
    }
    s.active_workers -= 1;
    s.current_file = None;
    let route = s.active_server_route.clone();
    let completed_batch = s
        .transfer
        .finish_item(TransferDirection::Upload, path, route);
    if completed_batch {
        s.completed_upload_batches = s.completed_upload_batches.saturating_add(1);
    }
    let total_handled = s.processed_count + s.failed_count;
    if total_handled >= s.total_queued && s.active_workers == 0 {
        apply_idle_state(&mut s, sync_index_ref);
        check_batch_notification(&mut batch_notify_ref.lock())
    } else {
        apply_active_state(&mut s, total_handled);
        None
    }
}

fn apply_idle_state(s: &mut AppState, sync_index_ref: &Arc<ShardedSyncIndex>) {
    s.queue_size = 0;
    s.status = if s.paused {
        "paused".to_string()
    } else {
        "idle".to_string()
    };
    s.progress = 100;
    log::info!("All {} file(s) processed. Idle.", s.total_queued);
    if let Err(err) = sync_index_ref.flush() {
        log::warn!("Failed to flush sync index on idle: {}", err);
    }
}

fn apply_active_state(s: &mut AppState, total_handled: usize) {
    s.queue_size = s.total_queued.saturating_sub(total_handled);
    s.progress = if s.total_queued > 0 {
        ((total_handled as f32 / s.total_queued as f32) * 100.0) as u8
    } else {
        0
    };
    s.status = "uploading".to_string();
}

fn check_batch_notification(batch_state: &mut BatchNotifyState) -> Option<u64> {
    if batch_state.active
        && batch_state.current_batch_id != batch_state.last_notified_batch_id
        && !batch_state.notify_scheduled
    {
        batch_state.notify_scheduled = true;
        Some(batch_state.current_batch_id)
    } else {
        None
    }
}

/// Spawn a debounced batch-notification task.
fn schedule_batch_notification(
    state_ref: Arc<parking_lot::Mutex<AppState>>,
    batch_notify_ref: Arc<parking_lot::Mutex<BatchNotifyState>>,
    batch_id: u64,
) {
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        let summary = batch_notification_summary(&state_ref, &batch_notify_ref, batch_id);
        if let Some((succeeded, failed)) = summary {
            notifications::send_sync_summary(succeeded, failed);
        }
    });
}

fn batch_notification_summary(
    state_ref: &Arc<parking_lot::Mutex<AppState>>,
    batch_notify_ref: &Arc<parking_lot::Mutex<BatchNotifyState>>,
    batch_id: u64,
) -> Option<(usize, usize)> {
    let s = state_ref.lock();
    let mut batch_state = batch_notify_ref.lock();
    if !batch_is_ready_to_notify(&s, &batch_state, batch_id) {
        clear_stale_batch_schedule(&mut batch_state, batch_id);
        return None;
    }
    let succeeded = s
        .processed_count
        .saturating_sub(batch_state.start_processed);
    let failed = s.failed_count.saturating_sub(batch_state.start_failed);
    batch_state.last_notified_batch_id = batch_id;
    batch_state.notify_scheduled = false;
    batch_state.active = false;
    Some((succeeded, failed))
}

fn batch_is_ready_to_notify(
    state: &AppState,
    batch_state: &BatchNotifyState,
    batch_id: u64,
) -> bool {
    let total_handled = state.processed_count + state.failed_count;
    let queue_idle = total_handled >= state.total_queued && state.active_workers == 0;
    batch_state.active
        && batch_state.notify_scheduled
        && batch_state.current_batch_id == batch_id
        && batch_state.current_batch_id != batch_state.last_notified_batch_id
        && queue_idle
}

fn clear_stale_batch_schedule(batch_state: &mut BatchNotifyState, batch_id: u64) {
    if batch_state.current_batch_id == batch_id {
        batch_state.notify_scheduled = false;
    }
}

fn activate_batch_if_needed(batch_state: &mut BatchNotifyState, state: &AppState) {
    if batch_state.active {
        return;
    }

    // Start a fresh notification batch when work is (re)introduced.
    batch_state.active = true;
    batch_state.notify_scheduled = false;
    batch_state.current_batch_id = batch_state.current_batch_id.saturating_add(1);
    batch_state.start_processed = state.processed_count;
    batch_state.start_failed = state.failed_count;
}

/// Return current epoch timestamp in seconds.
fn unix_timestamp_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Returns true if the current local clock hour falls inside the configured quiet window.
///
/// Wrapping windows (e.g. 23:00 to 06:00) are supported.
fn is_quiet_hour(start: Option<u8>, end: Option<u8>) -> bool {
    let (Some(start), Some(end)) = (start, end) else {
        return false;
    };
    let h = chrono::Local::now().hour() as u8;
    if start <= end {
        h >= start && h < end
    } else {
        // Wrapping window: e.g. 23 → 06
        h >= start || h < end
    }
}

/// Pause worker processing loops until environment checks and manual pauses permit resume.
async fn wait_until_allowed(
    state_ref: &Arc<parking_lot::Mutex<AppState>>,
    policy_ref: &Arc<parking_lot::Mutex<EnvironmentPolicy>>,
) {
    loop {
        let policy = *policy_ref.lock();
        let defer_reason = {
            let state = state_ref.lock();
            if state.paused {
                Some(
                    state
                        .pause_reason
                        .clone()
                        .unwrap_or_else(|| "Paused by user".to_string()),
                )
            } else if policy.pause_on_metered_network && runtime_env::is_metered_connection() {
                Some("Deferred on metered network".to_string())
            } else if policy.pause_on_battery_power && runtime_env::is_on_battery_power() {
                Some("Deferred while on battery power".to_string())
            } else if is_quiet_hour(policy.quiet_hours_start, policy.quiet_hours_end) {
                Some("Deferred during quiet hours".to_string())
            } else {
                None
            }
        };

        if let Some(reason) = defer_reason {
            {
                let mut state = state_ref.lock();
                state.status = "paused".to_string();
                state.pause_reason = Some(reason);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            continue;
        }

        let mut state = state_ref.lock();
        if state.status == "paused" && !state.paused {
            state.status = "idle".to_string();
            state.pause_reason = None;
        }
        break;
    }
}

/// Upload or reassociate a file, then ensure the resulting asset is present in the target album.
async fn handle_upload(
    api: &ImmichApiClient,
    task: &FileTask,
    progress: TransferProgressCallback,
) -> Option<SyncTarget> {
    // Pre-flight: ask the server whether this checksum already exists. When it
    // does, we skip streaming the file body entirely and reuse the existing
    // asset id for album assignment. The 409 DUPLICATE path below still acts
    // as a safety net for races (e.g. concurrent upload by another client).
    let asset_id = match api.find_existing_asset_id(&task.checksum).await {
        Some(existing) => {
            log::debug!("Asset already on server, skipping upload: {}", task.path);
            Some(existing)
        }
        None => {
            api.upload_asset(
                &task.path,
                &task.checksum,
                task.sidecar_path.as_deref(),
                Some(progress.clone()),
            )
            .await
        }
    };

    let asset_id = match resolve_final_asset_id(api, task, asset_id).await {
        Ok(id) => id,
        Err(early_return) => return early_return,
    };

    // Manual / library uploads skip album association entirely. The asset is
    // already on the server at this point; returning early avoids the
    // parent-dir-as-album fallback below.
    if task.skip_album {
        log::debug!(
            "Skipping album association for library upload: asset_id={}",
            asset_id
        );
        return Some(SyncTarget {
            album_name: None,
            album_id: None,
        });
    }

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

    let mut final_album_id = match resolve_album_id(api, task, &album_name).await {
        Some(id) => id,
        None => {
            log::warn!(
                "Could not resolve album '{}'. Asset uploaded but not added to album.",
                album_name
            );
            return None;
        }
    };

    final_album_id = match add_to_album_with_retry(api, task, &asset_id, final_album_id).await {
        Some(id) => id,
        None => return None,
    };

    Some(SyncTarget {
        album_name: Some(album_name),
        album_id: Some(final_album_id),
    })
}

/// Resolve the album ID from either an explicit task ID or by name lookup.
async fn resolve_album_id(
    api: &ImmichApiClient,
    task: &FileTask,
    album_name: &str,
) -> Option<String> {
    if let Some(ref id) = task.album_id
        && !id.is_empty()
    {
        return Some(id.clone());
    }
    match api.get_or_create_album(album_name).await {
        Ok(id) => id,
        Err(e) => {
            log::warn!("Failed to resolve album '{}': {}", album_name, e);
            None
        }
    }
}

/// Infer target album name from directory structure segment.
fn infer_album_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
}

/// Persist current list of failed items to retries cache.
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

/// Load historical list of failed tasks from retries cache.
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

async fn resolve_final_asset_id(
    api: &ImmichApiClient,
    task: &FileTask,
    asset_id: Option<String>,
) -> Result<String, Option<SyncTarget>> {
    let id = match asset_id {
        None => return Err(None),
        Some(ref id) if id == "DUPLICATE" => match api.find_existing_asset_id(&task.checksum).await
        {
            Some(existing) => existing,
            None => {
                log::info!("Asset already on server: {}", task.path);
                return Err(Some(SyncTarget {
                    album_name: task
                        .album_name
                        .clone()
                        .or_else(|| infer_album_name(&task.path)),
                    album_id: task.album_id.clone(),
                }));
            }
        },
        Some(id) => id,
    };
    Ok(id)
}

async fn add_to_album_with_retry(
    api: &ImmichApiClient,
    task: &FileTask,
    asset_id: &str,
    mut final_album_id: String,
) -> Option<String> {
    let asset_id_str = asset_id.to_string();
    if !api
        .add_assets_to_album(&final_album_id, std::slice::from_ref(&asset_id_str))
        .await
    {
        let name = task
            .album_name
            .clone()
            .or_else(|| infer_album_name(&task.path))?;
        log::warn!(
            "Album '{}' may be stale or deleted. Refreshing album resolution.",
            final_album_id
        );
        final_album_id = match api.resolve_album_by_name(&name, true).await {
            Ok(Some(id)) => id,
            Ok(None) | Err(_) => return None,
        };
        if !api
            .add_assets_to_album(&final_album_id, std::slice::from_ref(&asset_id_str))
            .await
        {
            return None;
        }
    }
    Some(final_album_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_manager::AppState;
    use parking_lot::Mutex;
    use std::collections::HashSet;
    use std::sync::Arc;
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
                policy: Arc::new(Mutex::new(EnvironmentPolicy {
                    pause_on_metered_network: false,
                    pause_on_battery_power: false,
                    quiet_hours_start: None,
                    quiet_hours_end: None,
                })),
                worker_limit: Arc::new(AtomicUsize::new(1)),
                batch_notify_state: Arc::new(Mutex::new(BatchNotifyState::default())),
                accepting_new: Arc::new(AtomicBool::new(true)),
                shutdown_token: CancellationToken::new(),
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
            skip_album: false,
            sidecar_path: None,
        };
        let js = serde_json::to_string(&task).unwrap();
        assert!(js.contains("sha123"));

        let deserialized: FileTask = serde_json::from_str(&js).unwrap();
        assert_eq!(deserialized.path, "/a/b.jpg");
        assert_eq!(deserialized.album_id.unwrap(), "id1");
    }

    #[test]
    fn test_filetask_skip_album_defaults_false_for_legacy_payloads() {
        // Retry-queue entries written before `skip_album` existed must still
        // deserialise — they keep the old watch-folder behaviour.
        let legacy_json = r#"{
            "path": "/a/b.jpg",
            "watch_path": "/a",
            "checksum": "sha123",
            "album_id": null,
            "album_name": null,
            "reassociate_only": false
        }"#;
        let task: FileTask = serde_json::from_str(legacy_json).unwrap();
        assert!(
            !task.skip_album,
            "legacy entries inherit album-creation behaviour"
        );
        assert!(
            task.sidecar_path.is_none(),
            "legacy entries default to no sidecar"
        );
    }

    #[test]
    fn test_filetask_skip_album_round_trips() {
        let task = FileTask {
            path: "/x.jpg".into(),
            watch_path: String::new(),
            checksum: "sha".into(),
            album_id: None,
            album_name: None,
            reassociate_only: false,
            skip_album: true,
            sidecar_path: None,
        };
        let js = serde_json::to_string(&task).unwrap();
        let restored: FileTask = serde_json::from_str(&js).unwrap();
        assert!(restored.skip_album);
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
            skip_album: false,
            sidecar_path: None,
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
            skip_album: false,
            sidecar_path: None,
        };

        assert!(qm.add_to_queue(task.clone()).await);
        assert!(!qm.add_to_queue(task.clone()).await);

        let queued = rx.recv().await.unwrap();
        assert_eq!(queued.path, task.path);
        assert!(pending_paths.lock().contains("/a/1.jpg"));
        assert_eq!(shared_state.lock().total_queued, 1);
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
            skip_album: false,
            sidecar_path: None,
        };

        retry_list.lock().push(task.clone());
        pending_paths.lock().insert(task.path.clone());
        shared_state.lock().failed_count = 1;

        assert!(qm.retry_failed_path(&task.path).await);

        let requeued = rx.recv().await.unwrap();
        assert_eq!(requeued.path, task.path);
        assert!(retry_list.lock().is_empty());

        let state = shared_state.lock();
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
            skip_album: false,
            sidecar_path: None,
        };

        retry_list.lock().push(task.clone());
        pending_paths.lock().insert(task.path.clone());
        shared_state.lock().failed_count = 1;

        assert_eq!(qm.clear_failed(), 1);
        assert!(retry_list.lock().is_empty());
        assert!(!pending_paths.lock().contains(&task.path));

        let state = shared_state.lock();
        assert_eq!(state.failed_count, 0);
        assert_eq!(state.recent_events[0].status, "cleared");
    }
}
