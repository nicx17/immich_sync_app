//! Immich API integration, connectivity failover, and album/cache helpers.

use reqwest::Client;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiIssue {
    pub summary: String,
    pub guidance: String,
}

pub struct ImmichApiClient {
    pub client: Client,
    pub internal_url: String,
    pub external_url: String,
    pub api_key: String,
    /// The currently active base URL selected by the last successful connectivity check.
    pub active_url: Mutex<Option<String>>,
    /// Most recent actionable API/client problem for the dashboard and diagnostics.
    last_issue: Mutex<Option<ApiIssue>>,
    /// Album name to album ID cache to avoid repeated list/create calls.
    album_cache: Mutex<HashMap<String, String>>,
    albums_fetched: Mutex<bool>,
}

impl ImmichApiClient {
    pub fn new(internal_url: String, external_url: String, api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(1) // keep at most 1 idle connection per host
            .pool_idle_timeout(Duration::from_secs(30)) // drop idle connections after 30s
            .build()
            .unwrap_or_default();

        let int = internal_url.trim_end_matches('/').to_string();
        let ext = external_url.trim_end_matches('/').to_string();

        log::debug!(
            "ImmichApiClient created: internal={}, external={}",
            int,
            ext
        );

        Self {
            client,
            internal_url: int,
            external_url: ext,
            api_key,
            active_url: Mutex::new(None),
            last_issue: Mutex::new(None),
            album_cache: Mutex::new(HashMap::new()),
            albums_fetched: Mutex::new(false),
        }
    }

    pub async fn active_route_label(&self) -> Option<String> {
        let active = self.active_url.lock().await.clone()?;
        Some(self.route_label_for_url(&active))
    }

    pub async fn latest_issue(&self) -> Option<ApiIssue> {
        self.last_issue.lock().await.clone()
    }

    async fn set_issue(&self, issue: ApiIssue) {
        *self.last_issue.lock().await = Some(issue);
    }

    async fn clear_issue(&self) {
        *self.last_issue.lock().await = None;
    }

    fn route_label_for_url(&self, url: &str) -> String {
        let trimmed = url.trim_end_matches('/');
        if !self.internal_url.is_empty() && trimmed == self.internal_url {
            "LAN".to_string()
        } else if !self.external_url.is_empty() && trimmed == self.external_url {
            "WAN".to_string()
        } else {
            "Custom".to_string()
        }
    }

    /// Determine which base URL to use, preferring the internal address when reachable.
    pub async fn check_connection(&self) -> bool {
        log::info!("Checking connectivity...");

        if self.ping_url(&self.internal_url).await {
            let mut active = self.active_url.lock().await;
            *active = Some(self.internal_url.clone());
            self.clear_issue().await;
            log::info!("Connected via LAN: {}", self.internal_url);
            return true;
        }

        if self.ping_url(&self.external_url).await {
            let mut active = self.active_url.lock().await;
            *active = Some(self.external_url.clone());
            self.clear_issue().await;
            log::info!("Connected via WAN: {}", self.external_url);
            return true;
        }

        log::error!("Could not connect to Immich server.");
        let mut active = self.active_url.lock().await;
        *active = None;
        self.set_issue(ApiIssue {
            summary: "Could not reach the Immich server".to_string(),
            guidance: "Check the LAN/WAN URLs, confirm the server is running, and verify your network connection."
                .to_string(),
        })
        .await;
        false
    }

    /// Ping a specific Immich base URL and validate that it returns a real `pong` response.
    pub async fn ping_url(&self, url: &str) -> bool {
        if url.is_empty() {
            return false;
        }
        let endpoint = format!("{}/api/server/ping", url.trim_end_matches('/'));
        log::debug!("Pinging: {}", endpoint);

        match self
            .client
            .get(&endpoint)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().as_u16() == 200 => {
                match resp.json::<serde_json::Value>().await {
                    Ok(json)
                        if json["res"].as_str().map(|s| s.to_lowercase())
                            == Some("pong".into()) =>
                    {
                        log::debug!("Ping success: {}", endpoint);
                        true
                    }
                    _ => {
                        log::warn!("Ping failed (not a valid Immich response): {}", endpoint);
                        false
                    }
                }
            }
            Ok(resp) => {
                log::warn!("Ping failed ({}): {}", resp.status(), endpoint);
                false
            }
            Err(e) => {
                log::warn!("Ping error ({}): {}", e, endpoint);
                false
            }
        }
    }

    /// Return the cached active base URL, resolving connectivity first if needed.
    async fn get_active_url(&self) -> Option<String> {
        {
            let active = self.active_url.lock().await;
            if active.is_some() {
                return active.clone();
            }
        }
        if self.check_connection().await {
            let active = self.active_url.lock().await;
            return active.clone();
        }
        None
    }

    /// Upload an asset to Immich.
    ///
    /// Returns the created asset ID on success, `None` on failure, or `"DUPLICATE"`
    /// when the server reports that the content already exists.
    pub async fn upload_asset(&self, file_path: &str, checksum: &str) -> Option<String> {
        let base_url = match self.get_active_url().await {
            Some(u) => u,
            None => {
                log::error!("No active connection. Skipping upload: {}", file_path);
                self.set_issue(ApiIssue {
                    summary: "No active server connection".to_string(),
                    guidance: "Test the server connection in Settings and confirm at least one Immich URL is reachable."
                        .to_string(),
                })
                .await;
                return None;
            }
        };

        let path = Path::new(file_path);
        if !path.exists() {
            log::warn!("File not found, skipping: {}", file_path);
            self.set_issue(ApiIssue {
                summary: "A queued file is no longer available".to_string(),
                guidance: "Check that the watched folder still exists and that the file was not moved or deleted before upload."
                    .to_string(),
            })
            .await;
            return None;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) => {
                log::error!("Could not read metadata for {}: {}", file_path, e);
                self.set_issue(ApiIssue {
                    summary: "Mimick could not read a queued file".to_string(),
                    guidance: "Verify folder permissions and make sure the file is still accessible to the app."
                        .to_string(),
                })
                .await;
                return None;
            }
        };

        let (created_at, modified_at) = file_timestamps_iso(&meta);
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "upload".to_string());
        let device_asset_id = format!("mimick-rust-{}", checksum);
        let device_id = "mimick-rust-client".to_string();
        let mime = mime_for_path(path);

        log::info!("Uploading: {} ({} bytes)", file_path, meta.len());
        log::debug!(
            "  device_asset_id={}, created={}",
            device_asset_id,
            created_at
        );

        // Stream the file body so large videos do not get buffered into memory.
        let file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) => {
                log::error!("Failed to open {}: {}", file_path, e);
                self.set_issue(ApiIssue {
                    summary: "Mimick could not open a queued file".to_string(),
                    guidance: "The file may be locked, deleted, or outside the app's allowed folder access."
                        .to_string(),
                })
                .await;
                return None;
            }
        };

        let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new());
        let file_body = reqwest::Body::wrap_stream(stream);

        let file_part = reqwest::multipart::Part::stream_with_length(file_body, meta.len())
            .file_name(filename.clone())
            .mime_str(mime)
            .ok()?;

        let form = reqwest::multipart::Form::new()
            .part("assetData", file_part)
            .text("deviceAssetId", device_asset_id)
            .text("deviceId", device_id)
            .text("fileCreatedAt", created_at)
            .text("fileModifiedAt", modified_at)
            .text("isFavorite", "false");

        let url = format!("{}/api/assets", base_url);

        match self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match status {
                    200 | 201 => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            let asset_id = json["id"].as_str().map(String::from);
                            self.clear_issue().await;
                            log::info!("Upload OK: {} => {:?}", filename, asset_id);
                            asset_id
                        } else {
                            log::warn!(
                                "Upload returned {} but body unreadable: {}",
                                status,
                                filename
                            );
                            None
                        }
                    }
                    409 => {
                        log::info!("Duplicate (already in Immich): {}", filename);
                        self.clear_issue().await;
                        // Some versions return the ID even on 409
                        if let Ok(json) = resp.json::<serde_json::Value>().await
                            && let Some(id) = json["id"].as_str()
                        {
                            return Some(id.to_string());
                        }
                        Some("DUPLICATE".to_string())
                    }
                    413 => {
                        log::error!("Upload failed (file too large): {}", filename);
                        // Reset active_url to force re-check
                        let mut active = self.active_url.lock().await;
                        *active = None;
                        self.set_issue(ApiIssue {
                            summary: "Immich rejected a file as too large".to_string(),
                            guidance: "Reduce the file size, raise the server's upload limits, or use a folder rule to skip oversized files."
                                .to_string(),
                        })
                        .await;
                        None
                    }
                    401 | 403 => {
                        self.set_issue(ApiIssue {
                            summary: "Immich rejected the API key".to_string(),
                            guidance: "Update the API key in Settings and make sure it has permission to upload assets."
                                .to_string(),
                        })
                        .await;
                        None
                    }
                    502..=504 => {
                        log::warn!("Server error {}: retrying later for {}", status, filename);
                        let mut active = self.active_url.lock().await;
                        *active = None;
                        self.set_issue(ApiIssue {
                            summary: "Immich is temporarily unavailable".to_string(),
                            guidance: "Wait a moment and retry. If it keeps happening, check the server logs and reverse proxy."
                                .to_string(),
                        })
                        .await;
                        None
                    }
                    _ => {
                        let body = resp.text().await.unwrap_or_default();
                        log::error!("Upload failed [{}] for {}: {}", status, filename, body);
                        self.set_issue(classify_http_issue(
                            RequestContext::Upload,
                            status,
                            Some(&filename),
                        ))
                        .await;
                        None
                    }
                }
            }
            Err(e) => {
                log::error!("Network error uploading {}: {}", filename, e);
                // Force connection re-check on next upload
                let mut active = self.active_url.lock().await;
                *active = None;
                self.set_issue(classify_network_issue(RequestContext::Upload, &e))
                    .await;
                None
            }
        }
    }

    // --------------- Album Management ---------------

    /// Get all albums from Immich, populating the local cache.
    async fn fetch_all_albums(&self) {
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
        log::info!("Fetching album list...");

        match self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(albums) = resp.json::<Vec<serde_json::Value>>().await {
                    let mut cache = self.album_cache.lock().await;
                    for album in &albums {
                        if let (Some(name), Some(id)) =
                            (album["albumName"].as_str(), album["id"].as_str())
                        {
                            cache.insert(name.to_string(), id.to_string());
                        }
                    }
                    *self.albums_fetched.lock().await = true;
                    self.clear_issue().await;
                    log::info!("Cached {} albums.", cache.len());
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
        {
            let mut cache = self.album_cache.lock().await;
            cache.clear();
        }
        *self.albums_fetched.lock().await = false;
        self.fetch_all_albums().await;
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

        log::info!("Creating album: '{}'", album_name);

        let body = serde_json::json!({
            "albumName": album_name,
            "description": "Created by Mimick"
        });

        match self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
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
            // Cannot fetch albums, so we shouldn't attempt to create one blindly, nor should we return Ok(None)
            // which implies the album doesn't exist and can't be created. It's a network error.
            return Err("Cannot fetch albums to verify existence".to_string());
        }
        self.create_album(album_name).await
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
    pub async fn find_existing_asset_id(&self, checksum: &str) -> Option<String> {
        let base_url = self.get_active_url().await?;
        let url = format!("{}/api/assets/bulk-upload-check", base_url);
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
            .header("x-api-key", &self.api_key)
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
        let body = serde_json::json!({ "ids": asset_ids });

        log::info!(
            "Adding {} asset(s) to album '{}'",
            asset_ids.len(),
            album_id
        );

        match self
            .client
            .put(&url)
            .header("x-api-key", &self.api_key)
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
}

#[derive(Debug, Clone, Copy)]
enum RequestContext {
    Upload,
    Albums,
    AlbumCreate,
    AlbumAssign,
}

fn classify_http_issue(context: RequestContext, status: u16, subject: Option<&str>) -> ApiIssue {
    match status {
        401 | 403 => ApiIssue {
            summary: "Immich rejected the API key".to_string(),
            guidance: "Update the API key in Settings and confirm it still has upload access."
                .to_string(),
        },
        404 if matches!(context, RequestContext::AlbumAssign | RequestContext::AlbumCreate) => {
            ApiIssue {
                summary: "An album reference is no longer valid".to_string(),
                guidance: "Refresh the album list or choose a different album before retrying."
                    .to_string(),
            }
        }
        413 => ApiIssue {
            summary: "Immich rejected a file as too large".to_string(),
            guidance: "Reduce the file size, raise the server upload limit, or skip oversized files with folder rules."
                .to_string(),
        },
        429 => ApiIssue {
            summary: "Immich rate-limited the request".to_string(),
            guidance: "Wait a moment and retry. If this happens often, lower upload concurrency or check reverse proxy limits."
                .to_string(),
        },
        502..=504 => ApiIssue {
            summary: "Immich is temporarily unavailable".to_string(),
            guidance: "Wait a moment and retry. If it keeps happening, inspect the server and reverse proxy logs."
                .to_string(),
        },
        _ => ApiIssue {
            summary: match context {
                RequestContext::Upload => {
                    format!("Immich could not accept {}", subject.unwrap_or("the upload"))
                }
                RequestContext::Albums => "Immich could not load the album list".to_string(),
                RequestContext::AlbumCreate => format!(
                    "Immich could not create album '{}'",
                    subject.unwrap_or("Unnamed")
                ),
                RequestContext::AlbumAssign => {
                    "Immich could not add the asset to the selected album".to_string()
                }
            },
            guidance: format!(
                "The server responded with HTTP {}. Check the server logs and retry after confirming the current configuration.",
                status
            ),
        },
    }
}

fn classify_network_issue(context: RequestContext, error: &reqwest::Error) -> ApiIssue {
    if error.is_timeout() {
        ApiIssue {
            summary: "The Immich request timed out".to_string(),
            guidance: "Check network quality and server responsiveness, then retry.".to_string(),
        }
    } else if error.is_connect() {
        ApiIssue {
            summary: "Could not reach the Immich server".to_string(),
            guidance: "Check the configured URLs, your network connection, and whether the server is online."
                .to_string(),
        }
    } else {
        ApiIssue {
            summary: match context {
                RequestContext::Upload => "The upload request failed before completion".to_string(),
                RequestContext::Albums => "The album request failed before completion".to_string(),
                RequestContext::AlbumCreate => {
                    "The album creation request failed before completion".to_string()
                }
                RequestContext::AlbumAssign => {
                    "The album assignment request failed before completion".to_string()
                }
            },
            guidance: "Retry the request after checking network connectivity and server health."
                .to_string(),
        }
    }
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("tiff") | Some("tif") => "image/tiff",
        _ => "application/octet-stream",
    }
}

fn file_timestamps_iso(meta: &std::fs::Metadata) -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let created = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(now);

    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(now);

    (unix_to_iso8601(created), unix_to_iso8601(modified))
}

/// Approximate ISO 8601 UTC from unix seconds (no chrono dependency).
fn unix_to_iso8601(secs: u64) -> String {
    // Days from epoch to year
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Gregorian calendar approximation
    let mut year = 1970u64;
    let mut rem_days = days;
    loop {
        let leap =
            (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
        let days_in_year = if leap { 366 } else { 365 };
        if rem_days < days_in_year {
            break;
        }
        rem_days -= days_in_year;
        year += 1;
    }
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let month_days: &[u64] = if leap {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &md in month_days {
        if rem_days < md {
            break;
        }
        rem_days -= md;
        month += 1;
    }
    let day = rem_days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        year, month, day, h, m, s
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_unix_to_iso8601() {
        assert_eq!(unix_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(unix_to_iso8601(1704067200), "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn test_mime_for_path() {
        assert_eq!(mime_for_path(Path::new("test.jpg")), "image/jpeg");
        assert_eq!(mime_for_path(Path::new("test.PNG")), "image/png");
        assert_eq!(mime_for_path(Path::new("test.mp4")), "video/mp4");
        assert_eq!(
            mime_for_path(Path::new("test.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_classify_http_issue_for_invalid_api_key() {
        let issue = classify_http_issue(RequestContext::Upload, 401, Some("photo.jpg"));
        assert_eq!(issue.summary, "Immich rejected the API key");
        assert!(issue.guidance.contains("API key"));
    }

    #[test]
    fn test_classify_http_issue_for_album_assign_404() {
        let issue = classify_http_issue(RequestContext::AlbumAssign, 404, Some("album-1"));
        assert_eq!(issue.summary, "An album reference is no longer valid");
    }

    #[tokio::test]
    async fn test_active_route_label_tracks_selected_url() {
        let client = ImmichApiClient::new(
            "http://lan.example".into(),
            "https://wan.example".into(),
            "token".into(),
        );
        *client.active_url.lock().await = Some("https://wan.example".into());

        assert_eq!(client.active_route_label().await.as_deref(), Some("WAN"));
    }
}
