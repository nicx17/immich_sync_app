//! Performs a startup catch-up scan for files that were missed while Mimick was not running.
//!
//! The scan runs in two stages:
//!   1. **Enumerate (parallel, sync)** -- uses `rayon` to walk all watch paths
//!      concurrently, collecting candidate files with their filesystem fingerprints.
//!   2. **Decide + queue (bounded async)** -- resolves album IDs, checks the sync
//!      index, hashes only when needed, and queues uploads with bounded concurrency.

use crate::api_client::ImmichApiClient;
use crate::app_context::AppContext;
use crate::config::FolderSyncMethod;
use crate::config::StartupCatchupMode;
use crate::config::WatchPathEntry;
use crate::library::album_sync;
use crate::monitor::{compute_sha1_chunked, is_supported_media_path, is_temporary_file};
use crate::queue_manager::{FileTask, QueueManager};
use crate::state_manager::AppState;
use crate::sync_index::{ShardedSyncIndex, SyncDecision, SyncTarget};
use futures_util::stream::{self, StreamExt};
use parking_lot::Mutex;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A file discovered during the parallel enumeration stage.
struct ScanCandidate {
    path: String,
    watch_path: String,
    album_name: String,
}

/// Scans watch folders at startup and queues new, changed, or retargeted files for upload.
///
/// Stage 1 parallelises the directory walk and filtering via rayon.
/// Stage 2 resolves album IDs and processes candidates with bounded async concurrency.
pub async fn queue_unsynced_files(
    watch_paths: Vec<WatchPathEntry>,
    queue_manager: Arc<QueueManager>,
    sync_index: Arc<ShardedSyncIndex>,
    api_client: Arc<ImmichApiClient>,
    catchup_mode: StartupCatchupMode,
    shared_state: Arc<Mutex<AppState>>,
    app_ctx: Arc<AppContext>,
) {
    if watch_paths.is_empty() {
        return;
    }

    // ── Stage 1: Parallel enumerate + fingerprint ────────────────────────
    //
    // Read last_sync once, before spawning into rayon, to avoid locking
    // shared_state on every file.
    let last_sync = shared_state
        .lock()
        .last_successful_sync_at
        .unwrap_or_default();

    let (candidates, mut seen_paths, skipped_enum, enum_errors) =
        enumerate_candidates(&watch_paths, catchup_mode, last_sync, &shared_state);

    // ── Stage 2: Async decide + queue ────────────────────────────────────

    // 2a. Pre-batch album ID resolution: collect unique names, resolve in parallel.
    let unique_albums: Vec<String> = candidates
        .iter()
        .map(|c| c.album_name.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let album_id_cache: Arc<Mutex<HashMap<String, Option<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Resolve album IDs with bounded concurrency.
    {
        let api = api_client.clone();
        let cache = album_id_cache.clone();
        stream::iter(unique_albums)
            .for_each_concurrent(8, |name| {
                let api = api.clone();
                let cache = cache.clone();
                async move {
                    match api.get_album_id_if_exists(&name).await {
                        Ok(id) => {
                            cache.lock().insert(name, id);
                        }
                        Err(err) => {
                            log::warn!("Startup scan: album lookup failed for '{}': {}", name, err);
                        }
                    }
                }
            })
            .await;
    }

    // 2b. Per-candidate: sync_decision -> hash (if needed) -> collect FileTask.
    //     Tasks are NOT yet queued; we batch a server-side checksum check next
    //     so files already on Immich never enter the upload pipeline.
    let prepared: Arc<Mutex<Vec<FileTask>>> = Arc::new(Mutex::new(Vec::new()));
    let skipped = Arc::new(AtomicUsize::new(skipped_enum));
    let errors = Arc::new(AtomicUsize::new(enum_errors));

    stream::iter(candidates)
        .for_each_concurrent(16, |candidate| {
            let sync_index = sync_index.clone();
            let api_client = api_client.clone();
            let album_cache = album_id_cache.clone();
            let prepared = prepared.clone();
            let skipped = skipped.clone();
            let errors = errors.clone();

            async move {
                let path = Path::new(&candidate.path);
                let album_name = candidate.album_name.clone();

                // Lookup existing album ID (no lock held across await).
                let existing_album_id = album_cache.lock().get(&album_name).cloned().flatten();

                let target = SyncTarget {
                    album_name: Some(album_name.clone()),
                    album_id: existing_album_id,
                };

                // sync_decision -- brief shard lock, no await.
                let decision = match sync_index.sync_decision(path, &target) {
                    Ok(d) => d,
                    Err(err) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        log::warn!(
                            "Startup scan could not inspect '{}': {}",
                            candidate.path,
                            err
                        );
                        return;
                    }
                };

                let (reassociate_only, cached_checksum) = match decision {
                    SyncDecision::UpToDate => {
                        skipped.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    SyncDecision::NeedsUpload => (false, None),
                    SyncDecision::NeedsReassociate => {
                        (true, sync_index.stored_checksum(&candidate.path))
                    }
                };

                let album_id = match resolve_album(&api_client, &album_name, &album_cache).await {
                    Ok(id) => id,
                    Err(err) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        log::warn!(
                            "Startup scan skipping '{}': album resolution failed: {}",
                            candidate.path,
                            err
                        );
                        return;
                    }
                };

                match hash_to_task(
                    candidate.path,
                    candidate.watch_path,
                    album_id,
                    Some(album_name),
                    reassociate_only,
                    cached_checksum,
                )
                .await
                {
                    Ok(task) => prepared.lock().push(task),
                    Err(()) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
        .await;

    // 2c. Batch pre-flight: ask the server which checksums it already has, so
    //     pre-existing assets bypass the upload queue entirely.
    let prepared_tasks: Vec<FileTask> = std::mem::take(&mut *prepared.lock());
    let unique_checksums: Vec<String> = prepared_tasks
        .iter()
        .map(|t| t.checksum.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let existing_on_server = if unique_checksums.is_empty() {
        HashMap::new()
    } else {
        api_client.bulk_existing_asset_ids(&unique_checksums).await
    };

    // 2d. Split tasks. Hits get reassociated inline (no upload); misses go to
    //     the upload queue.
    let mut to_reassociate: Vec<(FileTask, String)> = Vec::new();
    let mut to_upload: Vec<FileTask> = Vec::new();
    for task in prepared_tasks {
        match existing_on_server.get(&task.checksum) {
            Some(asset_id) => to_reassociate.push((task, asset_id.clone())),
            None => to_upload.push(task),
        }
    }

    let reassociated = Arc::new(AtomicUsize::new(0));
    stream::iter(to_reassociate)
        .for_each_concurrent(8, |(task, asset_id)| {
            let api = api_client.clone();
            let sync_index = sync_index.clone();
            let reassociated = reassociated.clone();
            async move {
                if let Some(ref album_id) = task.album_id
                    && !album_id.is_empty()
                {
                    let _ = api
                        .add_assets_to_album(album_id, std::slice::from_ref(&asset_id))
                        .await;
                }
                let target = SyncTarget {
                    album_name: task.album_name.clone(),
                    album_id: task.album_id.clone(),
                };
                if let Err(err) = sync_index.record_synced(&task.path, &task.checksum, &target) {
                    log::warn!(
                        "Could not record sync index for pre-existing asset '{}': {}",
                        task.path,
                        err
                    );
                }
                reassociated.fetch_add(1, Ordering::Relaxed);
            }
        })
        .await;

    let queued = Arc::new(AtomicUsize::new(0));
    for task in to_upload {
        if queue_manager.add_to_queue(task).await {
            queued.fetch_add(1, Ordering::Relaxed);
        }
    }

    trash_remote_assets_for_missing_local_files(
        &watch_paths,
        &seen_paths,
        sync_index.clone(),
        api_client.clone(),
    )
    .await;

    sync_album_to_folder_entries(&watch_paths, app_ctx).await;

    // Prune index entries for files that no longer exist. If a folder is
    // configured to mirror local deletions to the album, keep its missing
    // records so the next album sync can move the remote asset to trash.
    for entry in &watch_paths {
        let rules = entry.rules();
        if !rules.delete_folder_to_album {
            continue;
        }
        let root = Path::new(entry.path());
        for (path, _) in sync_index.records_under_path(root) {
            seen_paths.insert(path);
        }
    }
    if let Err(err) = sync_index.prune_missing(&seen_paths) {
        log::warn!("Failed to prune sync index after startup scan: {}", err);
    }

    let total_queued = queued.load(Ordering::Relaxed);
    let total_skipped = skipped.load(Ordering::Relaxed);
    let total_errors = errors.load(Ordering::Relaxed);
    let total_reassociated = reassociated.load(Ordering::Relaxed);

    if total_queued == 0 && total_reassociated == 0 {
        log::info!(
            "Startup scan complete: no unsynced files found ({} already current, {} error(s)).",
            total_skipped,
            total_errors
        );
        return;
    }

    log::info!(
        "Startup scan: queued={} reassociated={} skipped={} errors={}",
        total_queued,
        total_reassociated,
        total_skipped,
        total_errors
    );
}

async fn trash_remote_assets_for_missing_local_files(
    watch_paths: &[WatchPathEntry],
    seen_paths: &HashSet<String>,
    sync_index: Arc<ShardedSyncIndex>,
    api_client: Arc<ImmichApiClient>,
) {
    for entry in watch_paths {
        let rules = entry.rules();
        if !rules.delete_folder_to_album {
            continue;
        }

        let root = Path::new(entry.path());
        for (path, record) in sync_index.records_under_path(root) {
            if seen_paths.contains(&path) {
                continue;
            }

            let album_name = entry
                .album_name()
                .map(|name| name.to_string())
                .or(record.album_name.clone())
                .or_else(|| {
                    Path::new(&path)
                        .parent()
                        .and_then(|parent| parent.file_name())
                        .map(|name| name.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "Mimick".to_string());

            let configured_album_id = match entry {
                WatchPathEntry::WithConfig { album_id, .. } => album_id.clone(),
                WatchPathEntry::Simple(_) => None,
            };
            let Some(album_id) = configured_album_id.or(record.album_id.clone()) else {
                match api_client.get_album_id_if_exists(&album_name).await {
                    Ok(Some(album_id)) => {
                        if trash_remote_asset_by_checksum(
                            &api_client,
                            &sync_index,
                            &path,
                            &album_id,
                            &record.checksum,
                            &album_name,
                        )
                        .await
                        {
                            continue;
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        log::warn!(
                            "Startup deletion sync could not resolve album '{}': {}",
                            album_name,
                            err
                        );
                    }
                }
                continue;
            };

            trash_remote_asset_by_checksum(
                &api_client,
                &sync_index,
                &path,
                &album_id,
                &record.checksum,
                &album_name,
            )
            .await;
        }
    }
}

async fn sync_album_to_folder_entries(watch_paths: &[WatchPathEntry], app_ctx: Arc<AppContext>) {
    for entry in watch_paths {
        reconcile_entry(app_ctx.clone(), entry).await;
    }
}

/// Reconcile a single watch entry against its remote album: download new
/// items, trash local files removed from the album, and trash remote items
/// missing locally — gated by the entry's per-folder rules.
pub async fn reconcile_entry(app_ctx: Arc<AppContext>, entry: &WatchPathEntry) {
    let rules = entry.rules();
    let download_enabled = rules.sync_method != FolderSyncMethod::UploadOnly;
    if !download_enabled && !rules.delete_album_to_folder && !rules.delete_folder_to_album {
        return;
    }

    let watch_path = Path::new(entry.path()).to_path_buf();
    if !watch_path.is_dir() {
        return;
    }

    let album_name = entry
        .album_name()
        .map(|name| name.to_string())
        .or_else(|| {
            watch_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "Mimick".to_string());

    let configured_album_id = match entry {
        WatchPathEntry::WithConfig { album_id, .. } => album_id.clone(),
        WatchPathEntry::Simple(_) => None,
    };
    let album_id = match configured_album_id {
        Some(id) => Some(id),
        None => match app_ctx.api_client.get_album_id_if_exists(&album_name).await {
            Ok(id) => id,
            Err(err) => {
                log::warn!(
                    "Album-to-folder sync could not resolve album '{}': {}",
                    album_name,
                    err
                );
                None
            }
        },
    };
    let Some(album_id) = album_id else {
        return;
    };

    // Per-album lock: drop the reconcile if another instance (periodic or
    // manual) is already running for this album.
    let _guard = match app_ctx.reconcile_locks.try_acquire(album_id.clone()) {
        Some(g) => g,
        None => {
            log::debug!(
                "Reconcile already in progress for album {}, skipping",
                album_id
            );
            return;
        }
    };

    let diff = match album_sync::diff_album_vs_folder(
        app_ctx.clone(),
        &album_id,
        &watch_path,
        &rules,
        false,
    )
    .await
    {
        Ok(diff) => diff,
        Err(err) => {
            log::warn!(
                "Album-to-folder sync diff failed for '{}': {}",
                album_name,
                err
            );
            return;
        }
    };

    if !diff.to_download.is_empty() {
        let (downloaded, failed) = album_sync::execute_downloads(
            app_ctx.clone(),
            watch_path.clone(),
            Some(album_id.clone()),
            Some(album_name.clone()),
            diff.to_download,
        )
        .await;
        log::info!(
            "Album-to-folder sync for '{}' downloaded {} item(s), {} failure(s)",
            album_name,
            downloaded,
            failed
        );
    }

    if !diff.to_delete_local.is_empty() {
        let (trashed, failed) =
            album_sync::execute_local_deletions(app_ctx.clone(), diff.to_delete_local).await;
        log::info!(
            "Album-to-folder deletion sync for '{}' moved {} local item(s) to trash, {} failure(s)",
            album_name,
            trashed,
            failed
        );
    }

    if !diff.to_delete_remote.is_empty() {
        let count = diff.to_delete_remote.len();
        let trashed =
            album_sync::execute_remote_deletions(app_ctx.clone(), &album_id, diff.to_delete_remote)
                .await;
        log::info!(
            "Folder-to-album deletion sync for '{}' moved {} of {} remote item(s) to Immich trash",
            album_name,
            trashed,
            count
        );
    }
}

async fn trash_remote_asset_by_checksum(
    api_client: &ImmichApiClient,
    sync_index: &ShardedSyncIndex,
    local_path: &str,
    album_id: &str,
    checksum: &str,
    album_name: &str,
) -> bool {
    match find_album_asset_id_by_checksum(api_client, album_id, checksum).await {
        Ok(Some((asset_id, filename))) => {
            let ids = vec![asset_id];
            match api_client.delete_assets(&ids).await {
                Ok(()) => {
                    if let Err(err) = sync_index.remove_path(local_path) {
                        log::warn!(
                            "Startup deletion sync trashed '{}' in album '{}' but could not remove sync record for '{}': {}",
                            filename,
                            album_name,
                            local_path,
                            err
                        );
                    }
                    log::info!(
                        "Startup deletion sync moved '{}' in album '{}' to Immich trash",
                        filename,
                        album_name
                    );
                    true
                }
                Err(err) => {
                    log::warn!(
                        "Startup deletion sync could not move '{}' in album '{}' to trash: {}",
                        filename,
                        album_name,
                        err
                    );
                    false
                }
            }
        }
        Ok(None) => false,
        Err(err) => {
            log::warn!(
                "Startup deletion sync could not inspect album '{}': {}",
                album_name,
                err
            );
            false
        }
    }
}

async fn find_album_asset_id_by_checksum(
    api_client: &ImmichApiClient,
    album_id: &str,
    checksum: &str,
) -> Result<Option<(String, String)>, String> {
    let mut page = 1;
    loop {
        let (assets, has_more) = api_client
            .fetch_album_assets(album_id, page, 1000, None)
            .await?;
        if let Some(asset) = assets
            .into_iter()
            .find(|asset| asset.checksum.as_deref() == Some(checksum))
        {
            return Ok(Some((asset.id, asset.filename)));
        }
        if !has_more {
            return Ok(None);
        }
        page += 1;
    }
}

/// Stage 1: Walk all watch paths in parallel using rayon.
///
/// Returns `(candidates, seen_paths, skipped_count, error_count)`.
fn enumerate_candidates(
    watch_paths: &[WatchPathEntry],
    fallback_catchup_mode: StartupCatchupMode,
    last_sync: f64,
    shared_state: &Arc<Mutex<AppState>>,
) -> (Vec<ScanCandidate>, HashSet<String>, usize, usize) {
    let skipped = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);
    let seen_paths = Mutex::new(HashSet::new());

    let candidates: Vec<ScanCandidate> = watch_paths
        .par_iter()
        .flat_map(|entry| {
            if entry.sync_method() == FolderSyncMethod::DownloadOnly {
                return Vec::new();
            }

            let catchup_mode = entry.startup_catchup_mode(&fallback_catchup_mode);
            let watch_path_str = entry.path().to_string();
            let root = Path::new(&watch_path_str);
            if !root.exists() {
                log::warn!(
                    "Startup scan skipped missing watch path: {}",
                    root.display()
                );
                if let Some(mut state) = shared_state.try_lock() {
                    let status = state.folder_statuses.entry(watch_path_str).or_default();
                    status.last_error = Some("Permission lost or folder missing".to_string());
                }
                return Vec::new();
            }

            let mut results = Vec::new();
            let mut stack = vec![root.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let read_dir = match std::fs::read_dir(&dir) {
                    Ok(iter) => iter,
                    Err(err) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        log::warn!("Startup scan could not read '{}': {}", dir.display(), err);
                        continue;
                    }
                };

                for child in read_dir {
                    match child {
                        Ok(entry_fs) => {
                            let path = entry_fs.path();
                            if path.is_dir() {
                                stack.push(path);
                                continue;
                            }

                            if !is_supported_media_path(&path) {
                                continue;
                            }
                            if is_temporary_file(&path) || !entry.rules().matches(&path) {
                                continue;
                            }

                            let path_str = path.to_string_lossy().into_owned();

                            // Apply catchup-mode filtering.
                            if catchup_mode == StartupCatchupMode::RecentOnly {
                                if let Ok(meta) = entry_fs.metadata()
                                    && let Ok(modified) = meta.modified()
                                    && let Ok(duration) =
                                        std::time::SystemTime::now().duration_since(modified)
                                    && duration.as_secs() > 7 * 86400
                                {
                                    skipped.fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                            } else if catchup_mode == StartupCatchupMode::NewFilesOnly
                                && let Ok(meta) = entry_fs.metadata()
                                && let Ok(created) = meta.created().or_else(|_| meta.modified())
                            {
                                let created_secs = created
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs_f64();
                                if created_secs < last_sync {
                                    skipped.fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                            }

                            seen_paths.lock().insert(path_str.clone());
                            let album_name = effective_album_name(entry, &path);
                            results.push(ScanCandidate {
                                path: path_str,
                                watch_path: watch_path_str.clone(),
                                album_name,
                            });
                        }
                        Err(err) => {
                            errors.fetch_add(1, Ordering::Relaxed);
                            log::warn!("Startup scan directory entry error: {}", err);
                        }
                    }
                }
            }
            results
        })
        .collect();

    (
        candidates,
        seen_paths.into_inner(),
        skipped.into_inner(),
        errors.into_inner(),
    )
}

/// Hash a candidate file and submit it to the upload queue.
async fn hash_to_task(
    path: String,
    watch_path: String,
    album_id: Option<String>,
    album_name: Option<String>,
    reassociate_only: bool,
    checksum: Option<String>,
) -> Result<FileTask, ()> {
    let checksum = if let Some(checksum) = checksum {
        checksum
    } else {
        let path_for_hash = path.clone();
        match tokio::task::spawn_blocking(move || compute_sha1_chunked(&path_for_hash)).await {
            Ok(Ok(checksum)) => checksum,
            Ok(Err(err)) => {
                log::warn!("Startup scan could not checksum '{}': {}", path, err);
                return Err(());
            }
            Err(err) => {
                log::warn!("Startup scan checksum task failed for '{}': {}", path, err);
                return Err(());
            }
        }
    };

    Ok(FileTask {
        path,
        watch_path,
        checksum,
        album_id,
        album_name,
        reassociate_only,
    })
}

/// Resolve the effective album name for a file using per-folder configuration.
fn effective_album_name(entry: &WatchPathEntry, path: &Path) -> String {
    match entry.album_name() {
        Some(name) if !name.is_empty() && name != "Default (Folder Name)" => name.to_string(),
        _ => path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Mimick".to_string()),
    }
}

/// Resolve an album ID, creating the album if it doesn't exist yet.
/// Caches the result for reuse by other candidates in the same folder.
async fn resolve_album(
    api_client: &ImmichApiClient,
    album_name: &str,
    cache: &Arc<Mutex<HashMap<String, Option<String>>>>,
) -> Result<Option<String>, String> {
    // Fast path: already resolved.
    if let Some(Some(cached)) = cache.lock().get(album_name) {
        return Ok(Some(cached.clone()));
    }

    let resolved = api_client.resolve_album_by_name(album_name, false).await?;
    cache
        .lock()
        .insert(album_name.to_string(), resolved.clone());
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use crate::monitor::is_supported_media_path;
    use std::path::PathBuf;

    #[test]
    fn test_supported_media_path_filter() {
        assert!(is_supported_media_path(&PathBuf::from("image.avif")));
        assert!(is_supported_media_path(&PathBuf::from("image.jpg")));
        assert!(is_supported_media_path(&PathBuf::from("movie.mkv")));
        assert!(is_supported_media_path(&PathBuf::from("movie.mp4")));
        assert!(!is_supported_media_path(&PathBuf::from("notes.txt")));
    }
}
