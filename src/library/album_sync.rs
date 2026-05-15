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

    // Build a map of sync-index records whose local path no longer exists, keyed by
    // checksum. A local file that matches one of these is treated as a rename: we
    // rewrite the index in place instead of trashing the remote and re-uploading.
    let mut orphan_by_checksum: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (path, record) in ctx.sync_index.records_under_path(watch_path) {
        if !local_paths.contains(&path) {
            orphan_by_checksum.entry(record.checksum).or_insert(path);
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
        let was_previously_synced = ctx
            .sync_index
            .stored_checksum(&path_str)
            .is_some_and(|checksum| checksum == entry.checksum);

        if was_previously_synced {
            if remote_unhashed > 0 {
                log::debug!(
                    "Skipping local delete decision for {} because {} remote album item(s) have no checksum",
                    entry.local.path.display(),
                    remote_unhashed
                );
            } else if rules.delete_album_to_folder {
                to_delete_local.push(entry);
            }
            // Else: file was synced and is now absent from the album. Do not
            // re-upload — the user removed it intentionally.
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
        // Mark before we trash so the live monitor's deletion event finds the
        // path in the expected-self-deletions set even if our index cleanup
        // hasn't run yet. Suppresses the redundant remote-trash propagation.
        ctx.expected_self_deletions.mark(&path.to_string_lossy());
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
    // GIO's File::trash uses the Trash portal automatically when sandboxed, but
    // it fails on some FUSE-backed paths (notably document-portal handles like
    // /run/user/UID/doc/HANDLE/...) because the kernel-side trash spec can't
    // find a matching trash directory on that filesystem. Try GIO first for
    // compatibility, then fall back to invoking the Trash portal directly with
    // an opened file descriptor — the portal happily accepts portal-managed
    // files since they were granted R/W by the file-chooser.
    let gio_path = path.clone();
    let gio_result = tokio::task::spawn_blocking(move || {
        use gtk::gio::prelude::FileExt;
        let file = gtk::gio::File::for_path(gio_path);
        file.trash(gtk::gio::Cancellable::NONE)
            .map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?;

    if gio_result.is_ok() {
        return Ok(());
    }
    let gio_err = gio_result.err().unwrap_or_default();
    log::debug!(
        "GIO trash failed for {} ({}); trying portal",
        path.display(),
        gio_err
    );

    let portal_err = match trash_via_portal(&path).await {
        Ok(()) => return Ok(()),
        Err(err) => err,
    };
    log::debug!(
        "Trash portal failed for {} ({}); trying manual XDG trash",
        path.display(),
        portal_err
    );

    trash_via_manual_xdg(&path).await.map_err(|manual_err| {
        format!(
            "gio: {}; portal: {}; manual: {}",
            gio_err, portal_err, manual_err
        )
    })
}

async fn trash_via_portal(path: &Path) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| format!("open for trash: {}", err))?;
    let proxy = ashpd::desktop::trash::TrashProxy::new()
        .await
        .map_err(|err| format!("trash proxy: {}", err))?;
    proxy
        .trash_file(&std::os::fd::AsFd::as_fd(&file))
        .await
        .map_err(|err| format!("trash_file: {}", err))
}

/// XDG trash spec implementation as a last-resort fallback. Used when the
/// source file lives on a FUSE filesystem (typically document-portal mounts)
/// where g_file_trash gives up because it can't locate a per-filesystem trash
/// directory. The home-tier trash (~/.local/share/Trash) is writable from the
/// Flatpak sandbox, and the document portal allows unlinking files it granted
/// with R/W, so this path succeeds where GIO and the portal call do not.
async fn trash_via_manual_xdg(path: &Path) -> Result<(), String> {
    let source = path.to_path_buf();
    tokio::task::spawn_blocking(move || trash_manual_xdg_blocking(&source))
        .await
        .map_err(|err| err.to_string())?
}

fn trash_manual_xdg_blocking(source: &Path) -> Result<(), String> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .ok_or_else(|| "XDG_DATA_HOME unresolved".to_string())?;
    let trash_files = data_home.join("Trash").join("files");
    let trash_info = data_home.join("Trash").join("info");
    std::fs::create_dir_all(&trash_files).map_err(|err| format!("create Trash/files: {}", err))?;
    std::fs::create_dir_all(&trash_info).map_err(|err| format!("create Trash/info: {}", err))?;

    let original_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("trashed-file");
    let (basename, dest_file, dest_info) =
        reserve_trash_slot(&trash_files, &trash_info, original_name)?;

    if let Err(err) = std::fs::copy(source, &dest_file) {
        let _ = std::fs::remove_file(&dest_file);
        return Err(format!("copy to Trash/files: {}", err));
    }

    let deletion_date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let info_body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encode_path_for_trashinfo(source),
        deletion_date
    );
    if let Err(err) = std::fs::write(&dest_info, info_body) {
        let _ = std::fs::remove_file(&dest_file);
        return Err(format!("write Trash/info/{}.trashinfo: {}", basename, err));
    }

    if let Err(err) = std::fs::remove_file(source) {
        let _ = std::fs::remove_file(&dest_file);
        let _ = std::fs::remove_file(&dest_info);
        return Err(format!("unlink source: {}", err));
    }

    log::debug!("Trashed via manual XDG fallback: {}", source.display());
    Ok(())
}

fn reserve_trash_slot(
    files_dir: &Path,
    info_dir: &Path,
    original_name: &str,
) -> Result<(String, PathBuf, PathBuf), String> {
    for n in 0..10_000 {
        let candidate = if n == 0 {
            original_name.to_string()
        } else {
            let stem = Path::new(original_name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("trash");
            let ext = Path::new(original_name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if ext.is_empty() {
                format!("{}.{}", stem, n)
            } else {
                format!("{}.{}.{}", stem, n, ext)
            }
        };
        let file_path = files_dir.join(&candidate);
        let info_path = info_dir.join(format!("{}.trashinfo", candidate));
        if !file_path.exists() && !info_path.exists() {
            return Ok((candidate, file_path, info_path));
        }
    }
    Err("could not find unique trash slot after 10000 attempts".to_string())
}

/// Percent-encode bytes per RFC 3986 for the Path field of a .trashinfo file.
/// Slashes are preserved (the spec wants the absolute path readable).
fn encode_path_for_trashinfo(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        let safe = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/');
        if safe {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
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
