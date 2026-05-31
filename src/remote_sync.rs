//! Periodic remote-album reconciler.
//!
//! Re-runs the per-folder album↔folder diff on a fixed interval so changes
//! made directly in Immich (asset deletions, additions) propagate to local
//! folders without requiring an app restart.

use std::sync::Arc;
use std::time::Duration;

use crate::app_context::AppContext;
use crate::startup_scan::reconcile_entry;

const REMOTE_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Periodic task loop that performs remote-to-local and local-to-remote sync reconciliations.
pub async fn run_album_reconciler(ctx: Arc<AppContext>) {
    loop {
        tokio::time::sleep(REMOTE_POLL_INTERVAL).await;

        if !ctx.config.read().data.background_sync_enabled {
            continue;
        }
        if ctx.queue_manager.is_paused() {
            continue;
        }
        if !ctx.api_client.check_connection().await {
            continue;
        }

        let watch_paths = ctx.config.read().data.watch_paths.clone();
        for entry in &watch_paths {
            reconcile_entry(ctx.clone(), entry).await;
        }
    }
}

use crate::api_client::{ImmichApiClient, LibraryAsset};
use crate::config;

/// Description of a local deletion event request targeting the remote album.
#[derive(Clone, Debug)]
pub struct LocalDeletionRequest {
    /// Absolute local path of deleted asset.
    pub local_path: String,
    /// Target Immich asset identifier.
    pub asset_id: String,
    /// Absolute filename of deleted asset.
    pub asset_name: String,
    /// Mapped Immich album name.
    pub album_name: String,
    /// Mapped Immich album identifier.
    pub album_id: Option<String>,
}

/// Check if local deletion matches validation criteria for remote mirror sweep.
pub async fn build_local_deletion_request(
    ctx: Arc<AppContext>,
    path: String,
) -> Option<LocalDeletionRequest> {
    let record = match ctx.sync_index.record_for_path(&path) {
        Some(record) => record,
        None => {
            log::debug!("No sync record for deleted file: {}", path);
            return None;
        }
    };

    let path_obj = std::path::Path::new(&path);
    let entry = {
        let entries = ctx.live_watch_paths.lock();
        config::best_matching_watch_entry(path_obj, &entries).cloned()
    };
    let Some(entry) = entry else {
        log::debug!("Deleted file is not under any watch folder: {}", path);
        return None;
    };
    let rules = entry.rules();
    if !rules.delete_folder_to_album {
        log::debug!("Folder-to-album deletion disabled for: {}", path);
        return None;
    }

    let album_name = entry
        .album_name()
        .map(|name| name.to_string())
        .or(record.album_name.clone())
        .or_else(|| {
            path_obj
                .parent()
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "Mimick".to_string());

    let configured_album_id = match &entry {
        config::WatchPathEntry::WithConfig { album_id, .. } => album_id.clone(),
        config::WatchPathEntry::Simple(_) => None,
    };
    let album_id = match configured_album_id.or(record.album_id.clone()) {
        Some(id) => Some(id),
        None => match ctx.api_client.get_album_id_if_exists(&album_name).await {
            Ok(id) => id,
            Err(err) => {
                log::warn!(
                    "Could not resolve album '{}' for deletion sync: {}",
                    album_name,
                    err
                );
                None
            }
        },
    }?;

    match find_album_asset_by_checksum(ctx.api_client.clone(), &album_id, &record.checksum).await {
        Ok(Some(asset)) => Some(LocalDeletionRequest {
            local_path: path,
            asset_id: asset.id,
            asset_name: asset.filename,
            album_name,
            album_id: Some(album_id),
        }),
        Ok(None) => {
            log::debug!("No matching album asset for deleted file: {}", path);
            None
        }
        Err(err) => {
            log::warn!("Could not inspect album for deletion sync: {}", err);
            None
        }
    }
}

/// Mirrors local filesystem deletion by unlinking or trashing assets on Immich.
pub async fn trash_remote_after_local_delete(ctx: Arc<AppContext>, request: LocalDeletionRequest) {
    let asset_ids = vec![request.asset_id.clone()];
    let album_count = ctx
        .api_client
        .count_albums_for_asset(&request.asset_id)
        .await;
    let (succeeded, action_log) = match (album_count, request.album_id.as_deref()) {
        (Some(n), Some(album_id)) if n > 1 => {
            let ok = ctx
                .api_client
                .remove_assets_from_album(album_id, &asset_ids)
                .await;
            (
                ok,
                format!(
                    "Unlinked '{}' from album '{}' (asset belongs to {} albums; preserved on server)",
                    request.asset_name, request.album_name, n
                ),
            )
        }
        _ => match ctx.api_client.delete_assets(&asset_ids).await {
            Ok(()) => (
                true,
                format!(
                    "Mirrored local delete of '{}' to album '{}' (asset trashed on server)",
                    request.asset_name, request.album_name
                ),
            ),
            Err(err) => {
                log::warn!(
                    "Could not mirror local delete of '{}': {}; sync record kept for retry",
                    request.asset_name,
                    err
                );
                return;
            }
        },
    };
    if !succeeded {
        log::warn!(
            "Could not mirror local delete of '{}'; sync record kept for retry",
            request.asset_name
        );
        return;
    }
    if let Err(err) = ctx.sync_index.remove_path(&request.local_path) {
        log::warn!(
            "Server-side delete succeeded but sync record cleanup failed for '{}': {}",
            request.local_path,
            err
        );
    }
    log::info!("{}", action_log);
}

/// Iterate through album assets matching checksum to find matching Immich library record.
async fn find_album_asset_by_checksum(
    api_client: Arc<ImmichApiClient>,
    album_id: &str,
    checksum: &str,
) -> Result<Option<LibraryAsset>, String> {
    let mut page = 1;
    loop {
        let (assets, has_more) = api_client
            .fetch_album_assets(album_id, page, 1000, None)
            .await?;
        if let Some(asset) = assets
            .into_iter()
            .find(|asset| asset.checksum.as_deref() == Some(checksum))
        {
            return Ok(Some(asset));
        }
        if !has_more {
            return Ok(None);
        }
        page += 1;
    }
}
