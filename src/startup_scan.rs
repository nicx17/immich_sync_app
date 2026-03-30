//! Performs a startup catch-up scan for files that were missed while Mimick was not running.

use crate::api_client::ImmichApiClient;
use crate::config::StartupCatchupMode;
use crate::config::WatchPathEntry;
use crate::monitor::{compute_sha1_chunked, is_supported_media_path, is_temporary_file};
use crate::queue_manager::{FileTask, QueueManager};
use crate::state_manager::AppState;
use crate::sync_index::{SyncDecision, SyncIndex, SyncTarget};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Represents a file discovered during the startup scan and staged for queue submission.
#[derive(Clone)]
struct ScanCandidate {
    path: String,
    watch_path: String,
    album_id: Option<String>,
    album_name: Option<String>,
    reassociate_only: bool,
    checksum: Option<String>,
}

/// Scans watch folders at startup and queues new, changed, or retargeted files for upload.
pub async fn queue_unsynced_files(
    watch_paths: Vec<WatchPathEntry>,
    queue_manager: Arc<QueueManager>,
    sync_index: Arc<Mutex<SyncIndex>>,
    api_client: Arc<ImmichApiClient>,
    catchup_mode: StartupCatchupMode,
    shared_state: Arc<Mutex<AppState>>,
) {
    if watch_paths.is_empty() {
        return;
    }

    let mut seen_paths = HashSet::new();
    let mut candidates = Vec::new();
    let mut skipped_current = 0usize;
    let mut scan_errors = 0usize;
    let mut album_id_cache: HashMap<String, Option<String>> = HashMap::new();

    for entry in &watch_paths {
        let watch_path_str = entry.path().to_string();
        let root = Path::new(&watch_path_str);
        if !root.exists() {
            log::warn!(
                "Startup scan skipped missing watch path: {}",
                root.display()
            );
            if let Ok(mut state) = shared_state.lock() {
                let status = state.folder_statuses.entry(watch_path_str).or_default();
                status.last_error = Some("Permission lost or folder missing".to_string());
            }
            continue;
        }

        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let read_dir = match std::fs::read_dir(&dir) {
                Ok(iter) => iter,
                Err(err) => {
                    scan_errors += 1;
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

                        // Apply Startup Catch-up Controls
                        if catchup_mode == StartupCatchupMode::RecentOnly {
                            if let Ok(meta) = entry_fs.metadata()
                                && let Ok(modified) = meta.modified()
                                && let Ok(duration) =
                                    std::time::SystemTime::now().duration_since(modified)
                                && duration.as_secs() > 7 * 86400
                            {
                                skipped_current += 1;
                                continue;
                            }
                        } else if catchup_mode == StartupCatchupMode::NewFilesOnly {
                            let last_sync = shared_state
                                .lock()
                                .unwrap()
                                .last_successful_sync_at
                                .unwrap_or_default();

                            if let Ok(meta) = entry_fs.metadata()
                                && let Ok(created) = meta.created().or_else(|_| meta.modified())
                            {
                                let created_secs = created
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs_f64();
                                if created_secs < last_sync {
                                    skipped_current += 1;
                                    continue;
                                }
                            }
                        }

                        seen_paths.insert(path_str.clone());
                        let album_name = effective_album_name(entry, &path);
                        let album_id_result =
                            resolve_target_album_id(&api_client, &album_name, &mut album_id_cache)
                                .await;

                        let album_id = match album_id_result {
                            Ok(id) => id,
                            Err(err) => {
                                scan_errors += 1;
                                log::warn!(
                                    "Startup scan skipping '{}' because album resolution failed: {}",
                                    path.display(),
                                    err
                                );
                                continue;
                            }
                        };

                        let target = SyncTarget {
                            album_name: Some(album_name.clone()),
                            album_id: album_id.clone(),
                        };

                        let decision =
                            match sync_index.lock().unwrap().sync_decision(&path, &target) {
                                Ok(decision) => decision,
                                Err(err) => {
                                    scan_errors += 1;
                                    log::warn!(
                                        "Startup scan could not inspect '{}': {}",
                                        path.display(),
                                        err
                                    );
                                    continue;
                                }
                            };

                        match decision {
                            SyncDecision::UpToDate => {
                                skipped_current += 1;
                            }
                            SyncDecision::NeedsUpload => {
                                candidates.push(ScanCandidate {
                                    path: path_str,
                                    watch_path: watch_path_str.clone(),
                                    album_id,
                                    album_name: Some(album_name),
                                    reassociate_only: false,
                                    checksum: None,
                                });
                            }
                            SyncDecision::NeedsReassociate => {
                                let checksum =
                                    sync_index.lock().unwrap().stored_checksum(&path_str);
                                candidates.push(ScanCandidate {
                                    path: path_str,
                                    watch_path: watch_path_str.clone(),
                                    album_id,
                                    album_name: Some(album_name),
                                    reassociate_only: true,
                                    checksum,
                                });
                            }
                        }
                    }
                    Err(err) => {
                        scan_errors += 1;
                        log::warn!("Startup scan directory entry error: {}", err);
                    }
                }
            }
        }
    }

    if let Err(err) = sync_index.lock().unwrap().prune_missing(&seen_paths) {
        log::warn!("Failed to prune sync index after startup scan: {}", err);
    }

    if candidates.is_empty() {
        log::info!(
            "Startup scan complete: no unsynced files found ({} already current, {} error(s)).",
            skipped_current,
            scan_errors
        );
        return;
    }

    log::info!(
        "Startup scan found {} unsynced file(s) ({} already current, {} error(s)).",
        candidates.len(),
        skipped_current,
        scan_errors
    );

    let mut queued = 0usize;
    for candidate in candidates {
        let checksum = if let Some(checksum) = candidate.checksum.clone() {
            checksum
        } else {
            let path_for_hash = candidate.path.clone();
            match tokio::task::spawn_blocking(move || compute_sha1_chunked(&path_for_hash)).await {
                Ok(Ok(checksum)) => checksum,
                Ok(Err(err)) => {
                    log::warn!(
                        "Startup scan could not checksum '{}': {}",
                        candidate.path,
                        err
                    );
                    continue;
                }
                Err(err) => {
                    log::warn!(
                        "Startup scan checksum task failed for '{}': {}",
                        candidate.path,
                        err
                    );
                    continue;
                }
            }
        };

        if queue_manager
            .add_to_queue(FileTask {
                path: candidate.path,
                watch_path: candidate.watch_path,
                checksum,
                album_id: candidate.album_id,
                album_name: candidate.album_name,
                reassociate_only: candidate.reassociate_only,
            })
            .await
        {
            queued += 1;
        }
    }

    log::info!("Startup scan queued {} unsynced file(s).", queued);
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

/// Resolve and memoize target album IDs so repeated files in the same folder reuse the lookup.
async fn resolve_target_album_id(
    api_client: &ImmichApiClient,
    album_name: &str,
    album_id_cache: &mut HashMap<String, Option<String>>,
) -> Result<Option<String>, String> {
    if let Some(cached) = album_id_cache.get(album_name) {
        return Ok(cached.clone());
    }

    let resolved = api_client.resolve_album_by_name(album_name, false).await?;
    album_id_cache.insert(album_name.to_string(), resolved.clone());
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
