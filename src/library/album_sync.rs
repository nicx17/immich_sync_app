//! Album↔folder bidirectional sync diff and execution.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api_client::{LibraryAsset, TransferProgressCallback};
use crate::app_context::AppContext;
use crate::config::{FolderRules, FolderSyncMethod};
use crate::library::local_source::{LocalAsset, enumerate_local};
use crate::monitor::compute_sha1_chunked;
use crate::queue_manager::FileTask;
use crate::state_manager::TransferDirection;
use crate::sync_index::SyncTarget;

/// Gate album→folder deletion: portal trash currently fails on FUSE
/// document-portal paths (upstream bug). Flip to `true` when fixed.
const ALBUM_TO_FOLDER_TRASH_AVAILABLE: bool = false;

#[derive(Debug, Default, Clone)]
pub struct AlbumDiff {
    pub to_upload: Vec<LocalEntry>,
    pub to_download: Vec<LibraryAsset>,
    pub to_delete_remote: Vec<LibraryAsset>,
    pub to_delete_local: Vec<LocalEntry>,
    pub remote_unhashed: usize,
}

#[derive(Debug, Clone)]
pub struct LocalEntry {
    pub local: LocalAsset,
    pub checksum: String,
}

pub async fn diff_album_vs_folder(
    ctx: Arc<AppContext>,
    album_id: &str,
    watch_path: &Path,
    rules: &FolderRules,
    manual_sync: bool,
) -> Result<AlbumDiff, String> {
    let mut remote = Vec::new();
    let mut page: u32 = 1;
    loop {
        let (chunk, has_more) = ctx
            .api_client
            .fetch_album_assets(album_id, page, 1000, None)
            .await?;
        remote.extend(chunk);
        if !has_more {
            break;
        }
        page += 1;
    }

    let watch_root = watch_path.to_path_buf();
    let locals: Vec<LocalAsset> = enumerate_local(ctx.clone())
        .await
        .into_iter()
        .filter(|asset| asset.path.starts_with(&watch_root))
        .collect();

    let local_entries = resolve_local_checksums(ctx.clone(), locals).await;
    let local_set: HashSet<String> = local_entries.iter().map(|e| e.checksum.clone()).collect();
    let local_paths: HashSet<String> = local_entries
        .iter()
        .map(|e| e.local.path.to_string_lossy().to_string())
        .collect();

    // Orphan records (path gone) keyed by checksum — drives rename detection
    // and suppresses to_download for assets headed for to_delete_remote.
    // Skip records whose path still physically exists (filtered by rules,
    // unreadable subdir, or portal handle mismatch) — those aren't deletions.
    // Also skip records whose album_id is for a different album.
    let mut orphan_by_checksum: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (path, record) in ctx.sync_index.records_under_path(watch_path) {
        if local_paths.contains(&path) {
            continue;
        }
        if Path::new(&path).exists() {
            continue;
        }
        if record.album_id.as_deref().is_some_and(|id| id != album_id) {
            continue;
        }
        orphan_by_checksum.entry(record.checksum).or_insert(path);
    }

    let mut to_download = Vec::new();
    let mut remote_by_checksum = std::collections::HashMap::new();
    let mut remote_set = HashSet::new();
    let mut remote_unhashed = 0usize;
    for asset in &remote {
        match &asset.checksum {
            Some(c) if !c.is_empty() => {
                remote_set.insert(c.clone());
                remote_by_checksum
                    .entry(c.clone())
                    .or_insert_with(|| asset.clone());
                if !local_set.contains(c) {
                    // Skip if to_delete_remote will trash this asset (avoids
                    // download-then-trash conflict on retried deletions).
                    if rules.delete_folder_to_album && orphan_by_checksum.contains_key(c) {
                        continue;
                    }
                    to_download.push(asset.clone());
                }
            }
            _ => remote_unhashed += 1,
        }
    }

    let mut to_upload = Vec::new();
    let mut to_delete_local = Vec::new();
    for entry in local_entries {
        let path_str = entry.local.path.to_string_lossy().to_string();
        if remote_set.contains(&entry.checksum) {
            if let Some(old_path) = orphan_by_checksum.remove(&entry.checksum) {
                migrate_renamed_record(&ctx, &old_path, &path_str, &entry.checksum);
            }
            continue;
        }
        // "Previously synced to THIS album" — strict: a record with a
        // different album_id means the folder was re-targeted, treat as
        // never-synced (so we upload rather than trash).
        let was_previously_synced =
            ctx.sync_index
                .record_for_path(&path_str)
                .is_some_and(|record| {
                    record.checksum == entry.checksum
                        && record.album_id.as_deref().is_none_or(|id| id == album_id)
                });

        if was_previously_synced {
            if remote_unhashed > 0 {
                log::debug!(
                    "Skipping local delete decision for {} because {} remote album item(s) have no checksum",
                    entry.local.path.display(),
                    remote_unhashed
                );
            } else if rules.delete_album_to_folder && ALBUM_TO_FOLDER_TRASH_AVAILABLE {
                to_delete_local.push(entry);
            } else if manual_sync {
                // Manual sync only: surface as re-upload candidate so user can
                // explicitly restore the asset. Automatic context skips this.
                to_upload.push(entry);
            }
        } else {
            to_upload.push(entry);
        }
    }

    let mut to_delete_remote = Vec::new();
    if rules.delete_folder_to_album {
        let mut seen_remote_delete_ids = HashSet::new();
        for (path, record) in ctx.sync_index.records_under_path(watch_path) {
            if local_paths.contains(&path) {
                continue;
            }
            if Path::new(&path).exists() {
                continue;
            }
            if record.album_id.as_deref().is_some_and(|id| id != album_id) {
                continue;
            }
            if let Some(asset) = remote_by_checksum.get(&record.checksum) {
                if !seen_remote_delete_ids.insert(asset.id.clone()) {
                    continue;
                }
                to_delete_remote.push(asset.clone());
            }
        }
    }

    // Two-tick confirmation (#7) for mass deletes — small batches (≤5) trust
    // the single read. The previous "still on server" filter was removed
    // because bulk-upload-check matches even trashed assets, which silently
    // suppressed the common case of "user trashed asset from album".
    if to_delete_local.len() > 5 {
        let album = album_id.to_string();
        let pending = ctx.pending_deletions.clone();
        to_delete_local
            .retain(|entry| pending.confirm(&format!("local:{}:{}", album, entry.checksum)));
    }
    if to_delete_remote.len() > 5 {
        let album = album_id.to_string();
        let pending = ctx.pending_deletions.clone();
        to_delete_remote.retain(|asset| {
            let key = asset
                .checksum
                .as_deref()
                .map(|c| format!("remote:{}:{}", album, c))
                .unwrap_or_else(|| format!("remote:{}:id:{}", album, asset.id));
            pending.confirm(&key)
        });
    }

    // Clear pending-deletion confirmations for items that came back — both
    // for assets still in remote_set (alive in album) and local files that
    // are present (alive on disk).
    {
        let album = album_id.to_string();
        let pending = ctx.pending_deletions.clone();
        for checksum in &remote_set {
            pending.clear(&format!("local:{}:{}", album, checksum));
        }
        for entry in &to_upload {
            // Local files that were never previously synced — clear any stale
            // remote-trash confirmation for their checksum.
            pending.clear(&format!("remote:{}:{}", album, entry.checksum));
        }
        for path in &local_paths {
            // Records whose path is back on disk — clear remote-trash intent.
            if let Some(record) = ctx.sync_index.record_for_path(path) {
                pending.clear(&format!("remote:{}:{}", album, record.checksum));
            }
        }
    }

    if !to_upload.is_empty()
        || !to_download.is_empty()
        || !to_delete_local.is_empty()
        || !to_delete_remote.is_empty()
    {
        log::info!(
            "Album sync diff: upload={} download={} trash_local={} trash_remote={}",
            to_upload.len(),
            to_download.len(),
            to_delete_local.len(),
            to_delete_remote.len()
        );
    }

    if rules.sync_method == FolderSyncMethod::UploadOnly {
        to_download.clear();
    } else if rules.sync_method == FolderSyncMethod::DownloadOnly {
        to_upload.clear();
    }

    Ok(AlbumDiff {
        to_upload,
        to_download,
        to_delete_remote,
        to_delete_local,
        remote_unhashed,
    })
}

async fn resolve_local_checksums(ctx: Arc<AppContext>, locals: Vec<LocalAsset>) -> Vec<LocalEntry> {
    let mut out = Vec::with_capacity(locals.len());
    let mut to_compute: Vec<LocalAsset> = Vec::new();

    {
        for asset in locals {
            match ctx.sync_index.fresh_checksum(&asset.path) {
                Some(c) => out.push(LocalEntry {
                    local: asset,
                    checksum: c,
                }),
                None => to_compute.push(asset),
            }
        }
    }

    for asset in to_compute {
        let path_str = asset.path.to_string_lossy().to_string();
        let hashed = tokio::task::spawn_blocking(move || compute_sha1_chunked(&path_str))
            .await
            .map_err(|err| err.to_string())
            .and_then(|r| r.map_err(|err| err.to_string()));
        match hashed {
            Ok(checksum) => out.push(LocalEntry {
                local: asset,
                checksum,
            }),
            Err(err) => log::warn!("Skipping {} during diff: {}", asset.path.display(), err),
        }
    }

    out
}

fn migrate_renamed_record(ctx: &Arc<AppContext>, old_path: &str, new_path: &str, checksum: &str) {
    let target = ctx
        .sync_index
        .record_for_path(old_path)
        .map(|record| SyncTarget {
            album_name: record.album_name,
            album_id: record.album_id,
        })
        .unwrap_or_else(|| SyncTarget {
            album_name: None,
            album_id: None,
        });

    if let Err(err) = ctx.sync_index.remove_path(old_path) {
        log::warn!(
            "Could not migrate sync record from {} during rename: {}",
            old_path,
            err
        );
        return;
    }
    if let Err(err) = ctx.sync_index.record_synced(new_path, checksum, &target) {
        log::warn!(
            "Could not record sync entry for renamed file {}: {}",
            new_path,
            err
        );
        return;
    }
    log::debug!("Renamed: {} -> {}", old_path, new_path);
}

pub async fn execute_uploads(
    ctx: Arc<AppContext>,
    album_id: String,
    album_name: String,
    watch_path: PathBuf,
    entries: Vec<LocalEntry>,
) -> usize {
    let mut queued = 0;
    for entry in entries {
        let task = FileTask {
            path: entry.local.path.to_string_lossy().to_string(),
            watch_path: watch_path.to_string_lossy().to_string(),
            checksum: entry.checksum,
            album_id: Some(album_id.clone()),
            album_name: Some(album_name.clone()),
            reassociate_only: false,
        };
        if ctx.queue_manager.add_to_queue(task).await {
            queued += 1;
        }
    }
    queued
}

pub async fn execute_downloads(
    ctx: Arc<AppContext>,
    watch_path: PathBuf,
    album_id: Option<String>,
    album_name: Option<String>,
    assets: Vec<LibraryAsset>,
) -> (usize, usize) {
    let mut ok = 0;
    let mut failed = 0;
    {
        let mut state = ctx.state.lock();
        let route = state.active_server_route.clone();
        state.transfer.begin_group(
            TransferDirection::Download,
            Some(format!("{} album item(s)", assets.len())),
            route,
        );
    }
    for asset in assets {
        let safe_name =
            crate::sanitize::safe_filename(&asset.filename).unwrap_or_else(|| asset.id.clone());
        let dest = unique_destination(&watch_path, &safe_name);
        // Mark before the bytes land so the watcher's Create event finds the
        // path in the suppression set even if delivery beats our post-download
        // index write. The live monitor consumes the entry and skips queuing.
        ctx.expected_self_downloads.mark(&dest.to_string_lossy());
        let progress = album_download_progress(&ctx, asset.id.clone(), asset.filename.clone());
        match ctx
            .api_client
            .download_original_to_file(&asset.id, &dest, Some(progress))
            .await
        {
            Ok(_) => {
                if let Some(checksum) = asset.checksum.as_deref()
                    && let Err(err) = ctx.sync_index.record_synced(
                        &dest.to_string_lossy(),
                        checksum,
                        &SyncTarget {
                            album_name: album_name.clone(),
                            album_id: album_id.clone(),
                        },
                    )
                {
                    log::warn!(
                        "Downloaded {} but could not record sync index for {}: {}",
                        asset.filename,
                        dest.display(),
                        err
                    );
                }
                finish_album_download(&ctx, &asset.id);
                ok += 1
            }
            Err(err) => {
                finish_album_download(&ctx, &asset.id);
                log::warn!("Download {} ({}) failed: {}", asset.filename, asset.id, err);
                failed += 1;
            }
        }
    }
    (ok, failed)
}

pub async fn execute_remote_deletions(
    ctx: Arc<AppContext>,
    album_id: &str,
    assets: Vec<LibraryAsset>,
) -> usize {
    if assets.is_empty() {
        return 0;
    }
    // Ask the server how many albums each asset is a member of. If >1, the
    // asset lives in another album somewhere on Immich — unlink only from
    // this album, don't destroy it. If 1 (just this album) or unknown, trash
    // the asset. The fallback to trash on lookup failure preserves the prior
    // behaviour rather than silently doing nothing.
    let mut to_trash: Vec<String> = Vec::new();
    let mut to_unalbum: Vec<String> = Vec::new();
    for asset in &assets {
        let album_count = ctx.api_client.count_albums_for_asset(&asset.id).await;
        match album_count {
            Some(n) if n > 1 => to_unalbum.push(asset.id.clone()),
            _ => to_trash.push(asset.id.clone()),
        }
    }
    let mut ok = 0;
    if !to_trash.is_empty() {
        match ctx.api_client.delete_assets(&to_trash).await {
            Ok(()) => ok += to_trash.len(),
            Err(err) => log::warn!("Remote trash failed: {}", err),
        }
    }
    if !to_unalbum.is_empty()
        && ctx
            .api_client
            .remove_assets_from_album(album_id, &to_unalbum)
            .await
    {
        ok += to_unalbum.len();
    }
    ok
}

pub async fn execute_local_deletions(
    _ctx: Arc<AppContext>,
    entries: Vec<LocalEntry>,
) -> (usize, usize) {
    // Album-to-folder deletion (Mirror Album Deletions to Folder) is currently
    // disabled because the only acceptable trash path on Flatpak
    if !entries.is_empty() {
        log::warn!(
            "Album-to-folder deletion currently disabled (Flatpak portal trash limitation). \
             {} local file(s) left in place; sync_index records kept so a future tick can retry once trash is re-enabled.",
            entries.len()
        );
    }
    (0, 0)

    // Original implementation kept for reactivation:
    //
    // let mut ok = 0;
    // let mut failed = 0;
    // for entry in entries {
    //     let path = entry.local.path.clone();
    //     _ctx.expected_self_deletions.mark(&path.to_string_lossy());
    //     match move_to_trash(entry.local.path.clone()).await {
    //         Ok(()) => {
    //             if let Err(err) = _ctx.sync_index.remove_path(&path.to_string_lossy()) {
    //                 log::warn!(
    //                     "Local trash succeeded but sync index cleanup failed for {}: {}",
    //                     path.display(), err
    //                 );
    //             }
    //             ok += 1;
    //         }
    //         Err(err) => {
    //             log::warn!("Local trash operation failed for {}: {}", path.display(), err);
    //             failed += 1;
    //         }
    //     }
    // }
    // (ok, failed)
}

/// Portal-only trash. Single implementation, no fallbacks. Currently unused
#[allow(dead_code)]
async fn move_to_trash(path: PathBuf) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|err| format!("open for trash: {}", err))?;
    let proxy = ashpd::desktop::trash::TrashProxy::new()
        .await
        .map_err(|err| format!("trash proxy: {}", err))?;
    proxy
        .trash_file(&std::os::fd::AsFd::as_fd(&file))
        .await
        .map_err(|err| format!("trash_file: {}", err))
}

fn unique_destination(folder: &Path, filename: &str) -> PathBuf {
    let safe = crate::sanitize::safe_filename(filename).unwrap_or_else(|| "download".to_string());
    let mut candidate = folder.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    for n in 1..1000 {
        let alt = if ext.is_empty() {
            format!("{} ({})", stem, n)
        } else {
            format!("{} ({}).{}", stem, n, ext)
        };
        candidate = folder.join(alt);
        if !candidate.exists() {
            return candidate;
        }
    }
    candidate
}

fn album_download_progress(
    ctx: &Arc<AppContext>,
    item_id: String,
    item_label: String,
) -> TransferProgressCallback {
    let state_ref = ctx.state.clone();
    {
        let mut state = state_ref.lock();
        let route = state.active_server_route.clone();
        state.transfer.register_item(
            TransferDirection::Download,
            item_id.clone(),
            None,
            Some(item_label),
            route,
        );
    }

    Arc::new(move |bytes_done, total_bytes| {
        let mut state = state_ref.lock();
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
        let route = state.active_server_route.clone();
        state
            .transfer
            .update_item_bytes(TransferDirection::Download, &item_id, bytes_done, route);
    })
}

fn finish_album_download(ctx: &Arc<AppContext>, item_id: &str) {
    let mut state = ctx.state.lock();
    let route = state.active_server_route.clone();
    state
        .transfer
        .finish_item(TransferDirection::Download, item_id, route);
}
