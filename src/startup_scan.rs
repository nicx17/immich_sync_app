use crate::api_client::ImmichApiClient;
use crate::config::WatchPathEntry;
use crate::monitor::{compute_sha1_chunked, is_supported_media_path};
use crate::queue_manager::{FileTask, QueueManager};
use crate::sync_index::{SyncDecision, SyncIndex, SyncTarget};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ScanCandidate {
    path: String,
    album_id: Option<String>,
    album_name: Option<String>,
    reassociate_only: bool,
    checksum: Option<String>,
}

pub async fn queue_unsynced_files(
    watch_paths: Vec<WatchPathEntry>,
    queue_manager: Arc<QueueManager>,
    sync_index: Arc<Mutex<SyncIndex>>,
    api_client: Arc<ImmichApiClient>,
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
        let root = Path::new(entry.path());
        if !root.exists() {
            log::warn!(
                "Startup scan skipped missing watch path: {}",
                root.display()
            );
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

                        let path_str = path.to_string_lossy().into_owned();
                        seen_paths.insert(path_str.clone());
                        let album_name = effective_album_name(entry, &path);
                        let album_id = resolve_target_album_id(
                            &api_client,
                            &album_name,
                            &mut album_id_cache,
                        )
                        .await;
                        let target = SyncTarget {
                            album_name: Some(album_name.clone()),
                            album_id: album_id.clone(),
                        };

                        let decision = match sync_index.lock().unwrap().sync_decision(&path, &target) {
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
                                    album_id,
                                    album_name: Some(album_name),
                                    reassociate_only: false,
                                    checksum: None,
                                });
                            }
                            SyncDecision::NeedsReassociate => {
                                let checksum = sync_index
                                    .lock()
                                    .unwrap()
                                    .stored_checksum(&path_str);
                                candidates.push(ScanCandidate {
                                    path: path_str,
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

async fn resolve_target_album_id(
    api_client: &ImmichApiClient,
    album_name: &str,
    album_id_cache: &mut HashMap<String, Option<String>>,
) -> Option<String> {
    if let Some(cached) = album_id_cache.get(album_name) {
        return cached.clone();
    }

    let resolved = api_client.resolve_album_by_name(album_name, false).await;
    album_id_cache.insert(album_name.to_string(), resolved.clone());
    resolved
}

#[cfg(test)]
mod tests {
    use crate::monitor::is_supported_media_path;
    use std::path::PathBuf;

    #[test]
    fn test_supported_media_path_filter() {
        assert!(is_supported_media_path(&PathBuf::from("image.jpg")));
        assert!(is_supported_media_path(&PathBuf::from("movie.mp4")));
        assert!(!is_supported_media_path(&PathBuf::from("notes.txt")));
    }
}
