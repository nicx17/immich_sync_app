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
                    to_download.push(asset.clone());
                }
            }
            _ => remote_unhashed += 1,
        }
    }

    let mut to_upload = Vec::new();
    let mut to_delete_local = Vec::new();
    for entry in local_entries {
        if remote_set.contains(&entry.checksum) {
            continue;
        }

        let path_str = entry.local.path.to_string_lossy().to_string();
        let was_previously_synced = ctx
            .sync_index
            .stored_checksum(&path_str)
            .is_some_and(|checksum| checksum == entry.checksum);

        if rules.delete_album_to_folder && was_previously_synced && remote_unhashed == 0 {
            to_delete_local.push(entry);
        } else if was_previously_synced && remote_unhashed > 0 {
            log::debug!(
                "Skipping local delete/upload decision for {} because {} remote album item(s) have no checksum",
                entry.local.path.display(),
                remote_unhashed
            );
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
            if let Some(asset) = remote_by_checksum.get(&record.checksum) {
                if !seen_remote_delete_ids.insert(asset.id.clone()) {
                    continue;
                }
                to_delete_remote.push(asset.clone());
            }
        }
    }

    if rules.sync_method == FolderSyncMethod::UploadOnly {
        to_download.clear();
        to_delete_local.clear();
    } else if rules.sync_method == FolderSyncMethod::DownloadOnly {
        to_upload.clear();
        to_delete_remote.clear();
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
            let path_str = asset.path.to_string_lossy().to_string();
            match ctx.sync_index.stored_checksum(&path_str) {
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

pub async fn execute_remote_deletions(ctx: Arc<AppContext>, assets: Vec<LibraryAsset>) -> usize {
    let ids: Vec<String> = assets.into_iter().map(|asset| asset.id).collect();
    if ids.is_empty() {
        return 0;
    }
    match ctx.api_client.delete_assets(&ids).await {
        Ok(()) => ids.len(),
        Err(err) => {
            log::warn!("Remote trash operation failed: {}", err);
            0
        }
    }
}

pub async fn execute_local_deletions(
    ctx: Arc<AppContext>,
    entries: Vec<LocalEntry>,
) -> (usize, usize) {
    let mut ok = 0;
    let mut failed = 0;
    for entry in entries {
        let path = entry.local.path.clone();
        match move_to_trash(entry.local.path.clone()).await {
            Ok(()) => {
                if let Err(err) = ctx.sync_index.remove_path(&path.to_string_lossy()) {
                    log::warn!(
                        "Local trash succeeded but sync index cleanup failed for {}: {}",
                        path.display(),
                        err
                    );
                }
                ok += 1
            }
            Err(err) => {
                log::warn!(
                    "Local trash operation failed for {}: {}",
                    entry.local.path.display(),
                    err
                );
                failed += 1;
            }
        }
    }
    (ok, failed)
}

async fn move_to_trash(path: PathBuf) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        use gtk::gio::prelude::FileExt;
        let file = gtk::gio::File::for_path(path);
        file.trash(gtk::gio::Cancellable::NONE)
            .map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?
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
