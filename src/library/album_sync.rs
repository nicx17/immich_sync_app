//! Album↔folder bidirectional sync diff and execution.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api_client::LibraryAsset;
use crate::app_context::AppContext;
use crate::library::local_source::{LocalAsset, enumerate_local};
use crate::monitor::compute_sha1_chunked;
use crate::queue_manager::FileTask;

#[derive(Debug, Default, Clone)]
pub struct AlbumDiff {
    pub to_upload: Vec<LocalEntry>,
    pub to_download: Vec<LibraryAsset>,
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
) -> Result<AlbumDiff, String> {
    let mut remote = Vec::new();
    let mut page: u32 = 1;
    loop {
        let chunk = ctx
            .api_client
            .fetch_album_assets(album_id, page, 1000)
            .await?;
        let len = chunk.len();
        remote.extend(chunk);
        if len < 1000 {
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

    let mut to_download = Vec::new();
    let mut remote_set = HashSet::new();
    let mut remote_unhashed = 0usize;
    for asset in &remote {
        match &asset.checksum {
            Some(c) if !c.is_empty() => {
                remote_set.insert(c.clone());
                if !local_set.contains(c) {
                    to_download.push(asset.clone());
                }
            }
            _ => remote_unhashed += 1,
        }
    }

    let to_upload: Vec<LocalEntry> = local_entries
        .into_iter()
        .filter(|e| !remote_set.contains(&e.checksum))
        .collect();

    Ok(AlbumDiff {
        to_upload,
        to_download,
        remote_unhashed,
    })
}

async fn resolve_local_checksums(ctx: Arc<AppContext>, locals: Vec<LocalAsset>) -> Vec<LocalEntry> {
    let mut out = Vec::with_capacity(locals.len());
    let mut to_compute: Vec<LocalAsset> = Vec::new();

    {
        let index = ctx.sync_index.lock().unwrap();
        for asset in locals {
            let path_str = asset.path.to_string_lossy().to_string();
            match index.stored_checksum(&path_str) {
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
    assets: Vec<LibraryAsset>,
) -> (usize, usize) {
    let mut ok = 0;
    let mut failed = 0;
    for asset in assets {
        let dest = unique_destination(&watch_path, &asset.filename);
        match ctx
            .api_client
            .download_original_to_file(&asset.id, &dest)
            .await
        {
            Ok(_) => ok += 1,
            Err(err) => {
                log::warn!("Download {} ({}) failed: {}", asset.filename, asset.id, err);
                failed += 1;
            }
        }
    }
    (ok, failed)
}

fn unique_destination(folder: &Path, filename: &str) -> PathBuf {
    let mut candidate = folder.join(filename);
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
