//! Album management: list, create, cache, and add assets to albums.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use super::errors::{RequestContext, classify_http_issue, classify_network_issue};
use super::{AlbumSummary, ApiIssue, ImmichApiClient};

impl ImmichApiClient {
    async fn fetch_all_albums(&self) {
        let _fetch_guard = self.album_fetch_lock.lock().await;
        if *self.albums_fetched.lock().await {
            return;
        }
        self.fetch_all_albums_locked().await;
    }

    /// Inner fetch implementation. Assumes `album_fetch_lock` is held by the
    /// caller and `albums_fetched` is already known to be false.
    async fn fetch_all_albums_locked(&self) {
        let base_url = match self.get_active_url().await {
            Some(u) => u,
            None => {
                log::warn!("Cannot fetch albums: no active URL.");
                self.set_issue(ApiIssue {
                    summary: "Album list is unavailable".to_string(),
                    guidance: "Reconnect to the Immich server before refreshing albums."
                        .to_string(),
                })
                .await;
                return;
            }
        };

        let url = format!("{}/api/albums", base_url);
        let api_key = self.settings.read().api_key.clone();
        log::info!("Fetching album list...");

        match self
            .client
            .get(&url)
            .header("x-api-key", &api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(albums) = resp.json::<Vec<AlbumSummary>>().await {
                    let total = albums.len();
                    let mut fresh: HashMap<String, String> = HashMap::with_capacity(total);
                    let mut duplicates = 0usize;
                    for album in albums {
                        match fresh.get(&album.album_name) {
                            Some(existing_id) if existing_id != &album.id => {
                                duplicates += 1;
                                log::warn!(
                                    "Duplicate album on server: '{}' has both id {} and {} (keeping first). Future syncs will use the first.",
                                    album.album_name,
                                    existing_id,
                                    album.id
                                );
                            }
                            Some(_) => {
                                // Same name + same id: server returned an entry twice; ignore silently.
                            }
                            None => {
                                fresh.insert(album.album_name, album.id);
                            }
                        }
                    }
                    let unique = fresh.len();
                    {
                        let mut cache = self.album_cache.lock().await;
                        *cache = fresh;
                    }
                    *self.albums_fetched.lock().await = true;
                    self.clear_issue().await;
                    log::info!(
                        "Cached {} unique album(s) from {} server entries ({} duplicate(s) ignored).",
                        unique,
                        total,
                        duplicates
                    );
                }
            }
            Ok(resp) => {
                log::error!("Failed to fetch albums: {}", resp.status());
                self.set_issue(classify_http_issue(
                    RequestContext::Albums,
                    resp.status().as_u16(),
                    None,
                ))
                .await;
            }
            Err(e) => {
                log::error!("Network error fetching albums: {}", e);
                let mut active = self.active_url.lock().await;
                *active = None;
                self.set_issue(classify_network_issue(RequestContext::Albums, &e))
                    .await;
            }
        }
    }

    pub async fn refresh_album_cache(&self) {
        let _fetch_guard = self.album_fetch_lock.lock().await;
        {
            let mut cache = self.album_cache.lock().await;
            cache.clear();
        }
        *self.albums_fetched.lock().await = false;
        self.fetch_all_albums_locked().await;
    }

    /// Return a snapshot of all cached albums as a list of (albumName, id)
    pub async fn get_all_albums(&self) -> Result<Vec<(String, String)>, String> {
        if !*self.albums_fetched.lock().await {
            self.fetch_all_albums().await;
        }
        if !*self.albums_fetched.lock().await {
            return Err("Failed to fetch albums".to_string());
        }
        let cache = self.album_cache.lock().await;
        Ok(cache
            .iter()
            .map(|(n, id)| (n.clone(), id.clone()))
            .collect())
    }

    /// Create a new album. Returns the new album ID.
    pub async fn create_album(&self, album_name: &str) -> Result<Option<String>, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let url = format!("{}/api/albums", base_url);
        let api_key = self.settings.read().api_key.clone();

        log::info!("Creating album: '{}'", album_name);

        let body = serde_json::json!({
            "albumName": album_name,
            "description": "Created by Mimick"
        });

        match self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().as_u16() == 200 || resp.status().as_u16() == 201 => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let id = json["id"].as_str().map(String::from);
                    if let Some(id_str) = &id {
                        let mut cache = self.album_cache.lock().await;
                        cache.insert(album_name.to_string(), id_str.clone());
                    }
                    self.clear_issue().await;
                    log::info!("Album created: '{}' ({:?})", album_name, id);
                    Ok(id)
                } else {
                    Ok(None)
                }
            }
            Ok(resp) => {
                log::error!("Failed to create album '{}': {}", album_name, resp.status());
                self.set_issue(classify_http_issue(
                    RequestContext::AlbumCreate,
                    resp.status().as_u16(),
                    Some(album_name),
                ))
                .await;
                Err(format!("HTTP {}", resp.status()))
            }
            Err(e) => {
                log::error!("Network error creating album '{}': {}", album_name, e);
                self.set_issue(classify_network_issue(RequestContext::AlbumCreate, &e))
                    .await;
                Err(e.to_string())
            }
        }
    }

    /// Return an existing album ID or create a new one.
    pub async fn get_or_create_album(&self, album_name: &str) -> Result<Option<String>, String> {
        if !*self.albums_fetched.lock().await {
            self.fetch_all_albums().await;
        }
        {
            let cache = self.album_cache.lock().await;
            if let Some(id) = cache.get(album_name) {
                log::debug!("Album found in cache: '{}' ({})", album_name, id);
                return Ok(Some(id.clone()));
            }
        }
        if !*self.albums_fetched.lock().await {
            return Err("Cannot fetch albums to verify existence".to_string());
        }

        let create_lock = {
            let mut locks = self.album_create_locks.lock().await;
            locks
                .entry(album_name.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = create_lock.lock().await;

        {
            let cache = self.album_cache.lock().await;
            if let Some(id) = cache.get(album_name) {
                return Ok(Some(id.clone()));
            }
        }

        self.create_album(album_name).await
    }

    /// Return an existing album ID without creating a new album as a side effect.
    pub async fn get_album_id_if_exists(&self, album_name: &str) -> Result<Option<String>, String> {
        if !*self.albums_fetched.lock().await {
            self.fetch_all_albums().await;
        }
        if !*self.albums_fetched.lock().await {
            return Err("Cannot fetch albums to verify existence".to_string());
        }

        let cache = self.album_cache.lock().await;
        Ok(cache.get(album_name).cloned())
    }

    pub async fn resolve_album_by_name(
        &self,
        album_name: &str,
        force_refresh: bool,
    ) -> Result<Option<String>, String> {
        if force_refresh {
            self.refresh_album_cache().await;
        }
        self.get_or_create_album(album_name).await
    }

    /// Check whether an asset already exists on the server by checksum and return its asset ID.
    /// Batch checksum existence check. Returns a map from checksum to asset id
    /// for every checksum the server already has. Missing checksums are absent
    /// from the result. The Immich endpoint accepts arbitrary batch sizes, but
    /// we chunk to keep request bodies modest.
    pub async fn bulk_existing_asset_ids(&self, checksums: &[String]) -> HashMap<String, String> {
        const CHUNK: usize = 500;
        let mut out: HashMap<String, String> = HashMap::new();
        let Some(base_url) = self.get_active_url().await else {
            return out;
        };
        let url = format!("{}/api/assets/bulk-upload-check", base_url);
        let api_key = self.settings.read().api_key.clone();

        for chunk in checksums.chunks(CHUNK) {
            let assets: Vec<_> = chunk
                .iter()
                .map(|c| serde_json::json!({ "id": c, "checksum": c }))
                .collect();
            let body = serde_json::json!({ "assets": assets });

            match self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&body)
                .timeout(Duration::from_secs(30))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await
                        && let Some(results) = json["results"].as_array()
                    {
                        for item in results {
                            let id = item["id"].as_str();
                            let asset_id = item["assetId"].as_str();
                            if let (Some(checksum), Some(asset_id)) = (id, asset_id) {
                                out.insert(checksum.to_string(), asset_id.to_string());
                            }
                        }
                    }
                }
                Ok(resp) => log::warn!("Bulk upload check returned {}", resp.status()),
                Err(err) => log::warn!("Bulk upload check request failed: {}", err),
            }
        }

        out
    }

    pub async fn find_existing_asset_id(&self, checksum: &str) -> Option<String> {
        let base_url = self.get_active_url().await?;
        let url = format!("{}/api/assets/bulk-upload-check", base_url);
        let api_key = self.settings.read().api_key.clone();
        let body = serde_json::json!({
            "assets": [
                {
                    "id": checksum,
                    "checksum": checksum
                }
            ]
        });

        match self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let json = resp.json::<serde_json::Value>().await.ok()?;
                json["results"]
                    .as_array()
                    .and_then(|results| results.first())
                    .and_then(|item| item["assetId"].as_str())
                    .map(ToString::to_string)
            }
            Ok(resp) => {
                log::warn!(
                    "Bulk upload check failed for checksum {}: {}",
                    checksum,
                    resp.status()
                );
                None
            }
            Err(err) => {
                log::warn!(
                    "Bulk upload check request failed for checksum {}: {}",
                    checksum,
                    err
                );
                None
            }
        }
    }

    /// Add a list of asset IDs to an album.
    pub async fn add_assets_to_album(&self, album_id: &str, asset_ids: &[String]) -> bool {
        if album_id.is_empty() || asset_ids.is_empty() {
            log::warn!("Skipping add_assets_to_album: missing ID or assets.");
            return false;
        }

        let base_url = match self.get_active_url().await {
            Some(u) => u,
            None => return false,
        };

        let url = format!("{}/api/albums/{}/assets", base_url, album_id);
        let api_key = self.settings.read().api_key.clone();
        let body = serde_json::json!({ "ids": asset_ids });

        log::info!(
            "Adding {} asset(s) to album '{}'",
            asset_ids.len(),
            album_id
        );

        match self
            .client
            .put(&url)
            .header("x-api-key", &api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                log::info!("Assets added to album successfully.");
                self.clear_issue().await;
                true
            }
            Ok(resp) => {
                log::error!("Failed to add assets to album: {}", resp.status());
                self.set_issue(classify_http_issue(
                    RequestContext::AlbumAssign,
                    resp.status().as_u16(),
                    Some(album_id),
                ))
                .await;
                false
            }
            Err(e) => {
                log::error!("Network error adding assets to album: {}", e);
                self.set_issue(classify_network_issue(RequestContext::AlbumAssign, &e))
                    .await;
                false
            }
        }
    }

    /// Count the albums that currently contain the given asset on the
    /// server. Drives the trash-vs-remove-from-album decision when mirroring
    /// a local deletion: an asset in multiple albums should only be unlinked
    /// from the linked album, never destroyed.
    pub async fn count_albums_for_asset(&self, asset_id: &str) -> Option<usize> {
        let base_url = self.get_active_url().await?;
        let url = format!("{}/api/albums?assetId={}", base_url, asset_id);
        let api_key = self.settings.read().api_key.clone();
        match self
            .client
            .get(&url)
            .header("x-api-key", &api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let albums: Vec<serde_json::Value> = resp.json().await.ok()?;
                Some(albums.len())
            }
            Ok(resp) => {
                log::warn!(
                    "count_albums_for_asset({}) returned {}",
                    asset_id,
                    resp.status()
                );
                None
            }
            Err(err) => {
                log::warn!("count_albums_for_asset({}) failed: {}", asset_id, err);
                None
            }
        }
    }

    /// Remove assets from an album without trashing them on the server. Used
    /// when an asset is referenced from more than one watch folder — we want
    /// to mirror the local deletion's album side, not destroy the asset.
    pub async fn remove_assets_from_album(&self, album_id: &str, asset_ids: &[String]) -> bool {
        if album_id.is_empty() || asset_ids.is_empty() {
            return false;
        }
        let base_url = match self.get_active_url().await {
            Some(u) => u,
            None => return false,
        };
        let url = format!("{}/api/albums/{}/assets", base_url, album_id);
        let api_key = self.settings.read().api_key.clone();
        let body = serde_json::json!({ "ids": asset_ids });
        match self
            .client
            .delete(&url)
            .header("x-api-key", &api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                log::info!(
                    "Removed {} asset(s) from album '{}' (asset preserved on server)",
                    asset_ids.len(),
                    album_id
                );
                true
            }
            Ok(resp) => {
                log::warn!("Remove-from-album returned {}", resp.status());
                false
            }
            Err(err) => {
                log::warn!("Remove-from-album request failed: {}", err);
                false
            }
        }
    }
}
