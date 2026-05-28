//! Integrates with the Immich API, handles connectivity failover, and provides album/cache helpers.
//!
//! The client probes both an internal (LAN) and external (WAN) URL and
//! locks onto whichever responds first. Submodules split the API surface
//! by domain: `albums`, `library`, `search`, `upload`, and `errors`.
//! All HTTP requests share a single `reqwest::Client` connection pool.

use parking_lot::RwLock;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

mod albums;
mod errors;
mod library;
mod search;
mod upload;
mod upload_helpers;

#[cfg(test)]
use errors::{RequestContext, classify_http_issue};
#[cfg(test)]
use upload_helpers::{
    looks_like_iana_timezone, mime_for_path, normalize_file_timestamps, unix_to_utc_iso8601,
};

pub type TransferProgressCallback = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// Represents an actionable API or connection issue encountered during operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiIssue {
    /// Concise summary of the encountered issue.
    pub summary: String,
    /// Guidance text showing the user how to resolve it.
    pub guidance: String,
}

/// In-memory API client connection URLs and configuration settings.
#[derive(Debug, Clone)]
pub(super) struct ApiClientSettings {
    /// Server URL for local network connections.
    pub(super) internal_url: String,
    /// Server URL for external network connections.
    pub(super) external_url: String,
    /// Immich authorization API key.
    pub(super) api_key: String,
}

/// Simplified album summary response from Immich.
#[derive(Debug, serde::Deserialize)]
pub(super) struct AlbumSummary {
    /// Album identifier.
    pub(super) id: String,
    /// Name of the album.
    #[serde(rename = "albumName")]
    pub(super) album_name: String,
}

/// Detailed Immich library album representation.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct LibraryAlbum {
    /// Unique album identifier.
    pub id: String,
    /// Name of the album.
    #[serde(rename = "albumName")]
    pub album_name: String,
    /// Count of assets contained in the album.
    #[serde(rename = "assetCount")]
    pub asset_count: u32,
    /// Asset ID used as the album's cover thumbnail.
    #[serde(rename = "albumThumbnailAssetId")]
    pub thumbnail_asset_id: Option<String>,
    /// ISO 8601 creation timestamp.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// ISO 8601 modification timestamp.
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    /// User description of the album.
    #[serde(default)]
    pub description: String,
    /// Owner identifier.
    #[serde(rename = "ownerId", default)]
    pub owner_id: String,
    /// True if the album is shared.
    #[serde(default)]
    pub shared: bool,
}

/// Detailed Immich library asset representation.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct LibraryAsset {
    /// Unique asset identifier on the server.
    pub id: String,
    /// Original file name.
    #[serde(rename = "originalFileName")]
    pub filename: String,
    /// Original MIME type.
    #[serde(rename = "originalMimeType")]
    pub mime_type: String,
    /// Asset file creation timestamp.
    #[serde(rename = "fileCreatedAt")]
    pub created_at: String,
    /// Asset kind (e.g. `"IMAGE"` or `"VIDEO"`).
    #[serde(rename = "type")]
    pub asset_type: String,
    /// Thumbhash representation for preview blur effects.
    pub thumbhash: Option<String>,
    /// Display width in pixels.
    pub width: Option<f64>,
    /// Display height in pixels.
    pub height: Option<f64>,
    /// Canonical lowercase SHA-1 checksum.
    #[serde(default, deserialize_with = "deserialize_checksum_to_hex")]
    pub checksum: Option<String>,
    /// EXIF block; Immich keeps pixel dimensions here, not at the top level.
    #[serde(rename = "exifInfo", default)]
    pub exif_info: Option<ExifInfo>,
}

/// Immich returns asset checksums as base64-encoded SHA1, while Mimick computes
/// and stores them as lowercase hex. Normalize on deserialization so every
/// comparison site (album diff, deletion lookup, sync index) sees the same
/// canonical form.
fn deserialize_checksum_to_hex<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw: Option<String> = Option::deserialize(deserializer)?;
    Ok(raw.as_deref().and_then(normalize_checksum_to_hex))
}

pub fn normalize_checksum_to_hex(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() == 40 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(trimmed.to_ascii_lowercase());
    }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed.as_bytes())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(trimmed.as_bytes()))
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(trimmed.as_bytes()))
        .ok()?;
    if bytes.len() != 20 {
        return None;
    }
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Structured filters passed down search operations.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSearchFilters {
    /// Inferred or original file name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_name: Option<String>,
    /// User description or caption search query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Match against OCR-extracted text inside images. Distinct from
    /// `description` (user-set caption) — Immich indexes recognised text
    /// during ML processing and exposes it as its own filter dimension on
    /// both `MetadataSearchDto` and `SmartSearchDto`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr: Option<String>,
    /// `"IMAGE"` or `"VIDEO"`; `None` returns both.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub asset_type: Option<String>,
    /// ISO 8601 inclusive lower bound on `fileCreatedAt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taken_after: Option<String>,
    /// ISO 8601 inclusive upper bound on `fileCreatedAt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taken_before: Option<String>,
    /// Camera make.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    /// Camera model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Camera lens model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lens_model: Option<String>,
    /// City location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// State location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// City location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// True to only return favorited assets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    /// True to only return archived assets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
    /// True to only return motion photos.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_motion: Option<bool>,
    /// True to only return assets not associated with any albums.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_not_in_album: Option<bool>,
    /// True to only return assets having valid EXIF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_exif: Option<bool>,
    /// True to also return deleted assets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_deleted: Option<bool>,
    /// List of person identifiers to match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_ids: Option<Vec<String>>,
    /// List of tag identifiers to match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_ids: Option<Vec<String>>,
    /// Desired sorting order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<SortOrder>,
}

/// Direction of sort results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// Ascending order.
    Asc,
    /// Descending order.
    Desc,
}

/// Immich server asset count statistics.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct ServerStats {
    /// Number of image assets.
    pub images: u64,
    /// Number of video assets.
    pub videos: u64,
    /// Total assets.
    pub total: u64,
}

/// Immich server configuration and version details.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct ServerAbout {
    /// Semver string of the Immich instance.
    pub version: String,
}

/// Per-user usage row from `/api/server/statistics`.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageByUser {
    pub user_id: String,
    pub user_name: String,
    #[serde(default)]
    pub photos: u64,
    #[serde(default)]
    pub videos: u64,
    #[serde(default)]
    pub usage: u64,
    #[serde(default)]
    pub quota_size_in_bytes: Option<u64>,
}

/// Server-wide statistics (admin-only). Returned by `/api/server/statistics`.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatistics {
    #[serde(default)]
    pub photos: u64,
    #[serde(default)]
    pub videos: u64,
    #[serde(default)]
    pub usage: u64,
    #[serde(default)]
    pub usage_by_user: Vec<UsageByUser>,
}

/// A recognized person returned by Immich's facial recognition.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Person {
    /// Unique identifier of the person.
    pub id: String,
    /// Name assigned to the person.
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "isHidden")]
    pub is_hidden: bool,
}

/// List wrapper of recognized people returned by Immich.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct PeopleResponse {
    /// Internal people list array.
    pub(super) people: Vec<Person>,
}

/// Discovered item in an Immich explore page section.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExploreItem {
    /// Text value representing the item category (e.g. city or tag name).
    pub value: String,
    /// Associated representative asset for preview.
    pub data: LibraryAsset,
}

/// Discovered category section on the Immich explore page (e.g. places or tags).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExploreSection {
    /// Categorisation field name.
    #[serde(rename = "fieldName")]
    pub field_name: String,
    /// Discovered list of explore items.
    pub items: Vec<ExploreItem>,
}

/// Container matching assets returned alongside their EXIF metadata.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct AssetWithExif {
    /// Unique asset ID.
    pub(super) id: String,
    /// EXIF info details payload if populated.
    #[serde(rename = "exifInfo", default)]
    pub(super) exif_info: Option<ExifInfo>,
}

/// Search results response wrapper for EXIF queries.
#[derive(Debug, serde::Deserialize)]
pub(super) struct ExifSearchResponse {
    /// Contained assets list wrapper.
    pub(super) assets: ExifSearchAssets,
}

/// Asset items list returned inside an EXIF search response.
#[derive(Debug, serde::Deserialize)]
pub(super) struct ExifSearchAssets {
    /// Individual items within search page.
    pub(super) items: Vec<AssetWithExif>,
    /// Pagination pointer to the next search page, if any.
    #[serde(rename = "nextPage", default)]
    pub(super) next_page: Option<String>,
}

/// One city with a representative asset ID for thumbnail display.
/// Places display category representation holding the representative thumbnail asset.
pub struct PlaceItem {
    /// Name of the city location.
    pub city: String,
    /// Asset ID mapped as the representative cover image.
    pub asset_id: String,
}

/// Full EXIF metadata schema properties returned by Immich.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifInfo {
    /// Camera manufacturer name.
    #[serde(default)]
    pub make: Option<String>,
    /// Camera model.
    #[serde(default)]
    pub model: Option<String>,
    /// Camera lens model name.
    #[serde(default)]
    pub lens_model: Option<String>,
    /// F-number aperture value.
    #[serde(default)]
    pub f_number: Option<f64>,
    /// Focal length in millimeters.
    #[serde(default)]
    pub focal_length: Option<f64>,
    /// ISO speed rating.
    #[serde(default)]
    pub iso: Option<u32>,
    /// Shutter speed string representation.
    #[serde(default)]
    pub exposure_time: Option<String>,
    /// Uncompressed file size in bytes.
    #[serde(default)]
    pub file_size_in_byte: Option<u64>,
    /// Original capture datetime string.
    #[serde(default)]
    pub date_time_original: Option<String>,
    /// Discovered city name.
    #[serde(default)]
    pub city: Option<String>,
    /// Discovered state or region name.
    #[serde(default)]
    pub state: Option<String>,
    /// Discovered country name.
    #[serde(default)]
    pub country: Option<String>,
    /// GPS Latitude coordinate in decimal degrees.
    #[serde(default)]
    pub latitude: Option<f64>,
    /// GPS Longitude coordinate in decimal degrees.
    #[serde(default)]
    pub longitude: Option<f64>,
    /// Metadata description text.
    #[serde(default)]
    pub description: Option<String>,
    /// Image width in pixels.
    #[serde(default)]
    pub exif_image_width: Option<u32>,
    /// Image height in pixels.
    #[serde(default)]
    pub exif_image_height: Option<u32>,
}

/// Asset details metadata payload.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDetails {
    /// Extracted EXIF metadata block.
    #[serde(default)]
    pub exif_info: Option<ExifInfo>,
}

/// Type of asset thumbnails requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailSize {
    Thumbnail,
    Preview,
    /// Server-generated full-resolution copy; only present when the server
    /// has the "save full-size image" job enabled. 404s otherwise.
    Fullsize,
}

impl ThumbnailSize {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            ThumbnailSize::Thumbnail => "thumbnail",
            ThumbnailSize::Preview => "preview",
            ThumbnailSize::Fullsize => "fullsize",
        }
    }
}

/// Search response wrapper.
#[derive(Debug, serde::Deserialize)]
pub(super) struct SearchResponse {
    /// Inner assets block.
    pub(super) assets: SearchAssetSection,
}

/// Paginated asset list section within a search response.
#[derive(Debug, serde::Deserialize)]
pub(super) struct SearchAssetSection {
    /// Assets returned in the search.
    pub(super) items: Vec<LibraryAsset>,
    /// Authoritative pagination signal from Immich. Some(page) when more
    /// results exist, None when the search is exhausted. We only need its
    /// presence — has_more is computed as `next_page.is_some()`.
    #[serde(rename = "nextPage", default)]
    pub(super) next_page: Option<String>,
}

/// Asynchronous Immich API client with failover and request serialization.
pub struct ImmichApiClient {
    /// Internal HTTP client instance.
    pub client: Client,
    /// Configuration settings wrapper.
    settings: RwLock<ApiClientSettings>,
    /// The currently active base URL, selected by the last successful connectivity check.
    pub active_url: Mutex<Option<String>>,
    /// Most recent actionable API/client problem, used for the dashboard and diagnostics.
    last_issue: Mutex<Option<ApiIssue>>,
    /// Caches album names to album IDs to avoid repeated list/create API calls.
    album_cache: Mutex<HashMap<String, String>>,
    /// Per-album-name async locks that serialize concurrent get-or-create calls
    /// for the *same* name, preventing duplicate-album creation under load.
    album_create_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Serializes `fetch_all_albums` so concurrent callers don't all hit the
    /// network and race writes into the cache.
    album_fetch_lock: Mutex<()>,
    /// Serializes `check_connection` so concurrent callers collapse into one
    /// connectivity probe instead of each issuing their own LAN+WAN pings.
    connection_check_lock: Mutex<()>,
    /// Timestamp of the most recent successful connectivity probe. Used to
    /// coalesce concurrent burst callers without suppressing periodic re-checks.
    last_successful_check: Mutex<Option<Instant>>,
    /// Flag indicating whether the album list has been successfully fetched in this session.
    albums_fetched: Mutex<bool>,
    /// Semaphore guarding maximum concurrent thumbnail downloads.
    thumbnail_semaphore: Arc<Semaphore>,
}

impl ImmichApiClient {
    /// Initialize a new ImmichApiClient.
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
            settings: RwLock::new(ApiClientSettings {
                internal_url: int,
                external_url: ext,
                api_key,
            }),
            active_url: Mutex::new(None),
            last_issue: Mutex::new(None),
            album_cache: Mutex::new(HashMap::new()),
            album_create_locks: Mutex::new(HashMap::new()),
            album_fetch_lock: Mutex::new(()),
            connection_check_lock: Mutex::new(()),
            last_successful_check: Mutex::new(None),
            albums_fetched: Mutex::new(false),
            thumbnail_semaphore: Arc::new(Semaphore::new(8)),
        }
    }

    /// Retrieve the route label (LAN/WAN) for the currently active connection URL.
    pub async fn active_route_label(&self) -> Option<String> {
        let active = self.active_url.lock().await.clone()?;
        Some(self.route_label_for_url(&active))
    }

    /// Retrieve the most recently recorded API issue.
    pub async fn latest_issue(&self) -> Option<ApiIssue> {
        self.last_issue.lock().await.clone()
    }

    /// Update in-memory configuration settings and reset connection state.
    pub async fn update_settings(
        &self,
        internal_url: String,
        external_url: String,
        api_key: String,
    ) {
        {
            let mut settings = self.settings.write();
            settings.internal_url = internal_url.trim_end_matches('/').to_string();
            settings.external_url = external_url.trim_end_matches('/').to_string();
            settings.api_key = api_key;
        }

        *self.active_url.lock().await = None;
        self.refresh_album_cache().await;
        self.clear_issue().await;
    }

    /// Set the last encountered API issue.
    pub(super) async fn set_issue(&self, issue: ApiIssue) {
        *self.last_issue.lock().await = Some(issue);
    }

    /// Clear the active API issue.
    pub(super) async fn clear_issue(&self) {
        *self.last_issue.lock().await = None;
    }

    /// Retrieve a snapshot copy of the current configuration settings.
    pub(super) fn settings_snapshot(&self) -> ApiClientSettings {
        self.settings.read().clone()
    }

    /// Retrieve the LAN/WAN/Custom route label matching a specific URL.
    pub(super) fn route_label_for_url(&self, url: &str) -> String {
        let settings = self.settings.read().clone();
        let trimmed = url.trim_end_matches('/');
        if !settings.internal_url.is_empty() && trimmed == settings.internal_url {
            "LAN".to_string()
        } else if !settings.external_url.is_empty() && trimmed == settings.external_url {
            "WAN".to_string()
        } else {
            "Custom".to_string()
        }
    }

    /// Determine which base URL to use, preferring the internal address when reachable.
    pub async fn check_connection(&self) -> bool {
        let _check_guard = self.connection_check_lock.lock().await;

        // Coalesce burst callers: if another caller probed successfully within
        // the last second, reuse that result. Periodic re-checks (e.g. the 5s
        // library ping loop) still re-probe because their gap exceeds 1s.
        if self.active_url.lock().await.is_some()
            && let Some(when) = *self.last_successful_check.lock().await
            && when.elapsed() < Duration::from_secs(1)
        {
            return true;
        }

        log::debug!("Checking connectivity...");
        let settings = self.settings.read().clone();
        let was_active = self.active_url.lock().await.clone();

        if self.ping_url(&settings.internal_url).await {
            let mut active = self.active_url.lock().await;
            let was_offline = was_active.is_none();
            *active = Some(settings.internal_url.clone());
            *self.last_successful_check.lock().await = Some(Instant::now());
            self.clear_issue().await;
            if was_offline {
                log::info!("Connected via LAN: {}", settings.internal_url);
            } else {
                log::debug!("Connected via LAN: {}", settings.internal_url);
            }
            return true;
        }

        if self.ping_url(&settings.external_url).await {
            let mut active = self.active_url.lock().await;
            let was_offline = was_active.is_none();
            *active = Some(settings.external_url.clone());
            *self.last_successful_check.lock().await = Some(Instant::now());
            self.clear_issue().await;
            if was_offline {
                log::info!("Connected via WAN: {}", settings.external_url);
            } else {
                log::debug!("Connected via WAN: {}", settings.external_url);
            }
            return true;
        }

        let mut active = self.active_url.lock().await;
        let was_online = active.is_some();
        *active = None;
        if was_online {
            log::error!("Could not connect to Immich server.");
        } else {
            log::debug!("Server still unreachable.");
        }
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
    pub(super) async fn get_active_url(&self) -> Option<String> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn normalize_checksum_passes_through_lowercase_hex() {
        let hex = "823cb0e790d643a90f61f07e5c2bdd588cf8f230";
        assert_eq!(normalize_checksum_to_hex(hex), Some(hex.to_string()));
    }

    #[test]
    fn normalize_checksum_lowercases_uppercase_hex() {
        let hex_up = "823CB0E790D643A90F61F07E5C2BDD588CF8F230";
        assert_eq!(
            normalize_checksum_to_hex(hex_up),
            Some(hex_up.to_ascii_lowercase())
        );
    }

    #[test]
    fn normalize_checksum_decodes_base64_to_hex() {
        // Base64 of the 20-byte SHA1 0x82 0x3c ... 0x30
        let b64 = "gjyw55DWQ6kPYfB+XCvdWIz48jA=";
        let expected = "823cb0e790d643a90f61f07e5c2bdd588cf8f230";
        assert_eq!(normalize_checksum_to_hex(b64), Some(expected.to_string()));
    }

    #[test]
    fn normalize_checksum_rejects_empty_and_unknown() {
        assert_eq!(normalize_checksum_to_hex(""), None);
        assert_eq!(normalize_checksum_to_hex("not-a-checksum"), None);
    }

    #[test]
    fn library_asset_deserializes_base64_checksum_as_hex() {
        let json = serde_json::json!({
            "id": "asset-1",
            "originalFileName": "a.png",
            "originalMimeType": "image/png",
            "fileCreatedAt": "2024-01-01T00:00:00.000Z",
            "type": "IMAGE",
            "checksum": "gjyw55DWQ6kPYfB+XCvdWIz48jA="
        });
        let asset: LibraryAsset = serde_json::from_value(json).unwrap();
        assert_eq!(
            asset.checksum.as_deref(),
            Some("823cb0e790d643a90f61f07e5c2bdd588cf8f230")
        );
    }

    #[test]
    fn test_unix_to_utc_iso8601() {
        assert_eq!(unix_to_utc_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(unix_to_utc_iso8601(1704067200), "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn test_mime_for_path() {
        assert_eq!(mime_for_path(Path::new("test.avif")), "image/avif");
        assert_eq!(mime_for_path(Path::new("test.jpg")), "image/jpeg");
        assert_eq!(mime_for_path(Path::new("test.jpe")), "image/jpeg");
        assert_eq!(mime_for_path(Path::new("test.heif")), "image/heif");
        assert_eq!(mime_for_path(Path::new("test.jp2")), "image/jp2");
        assert_eq!(mime_for_path(Path::new("test.jxl")), "image/jxl");
        assert_eq!(mime_for_path(Path::new("test.PNG")), "image/png");
        assert_eq!(
            mime_for_path(Path::new("test.psd")),
            "image/vnd.adobe.photoshop"
        );
        assert_eq!(mime_for_path(Path::new("test.svg")), "image/svg+xml");
        assert_eq!(mime_for_path(Path::new("test.mp4")), "video/mp4");
        assert_eq!(mime_for_path(Path::new("test.insv")), "video/mp4");
        assert_eq!(mime_for_path(Path::new("test.mkv")), "video/x-matroska");
        assert_eq!(mime_for_path(Path::new("test.mxf")), "application/mxf");
        assert_eq!(
            mime_for_path(Path::new("test.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_mime_for_path_covers_immich_spec() {
        // Every extension from pv_docs/Library_view_feature.md must map to a
        // non-fallback MIME so uploads/downloads pick the right pipeline.
        const SPEC_EXTENSIONS: &[&str] = &[
            // RAW
            "3fr", "ari", "arw", "cap", "cin", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff",
            "iiq", "k25", "kdc", "mrw", "nef", "nrw", "orf", "ori", "pef", "psd", "raf", "raw",
            "rw2", "rwl", "sr2", "srf", "srw", "x3f", // Web image
            "avif", "bmp", "gif", "jpeg", "jpg", "png", "webp", // Other image
            "heic", "heif", "hif", "insp", "jp2", "jpe", "jxl", "svg", "tif", "tiff",
            // Video
            "3gp", "3gpp", "avi", "flv", "insv", "m2t", "m2ts", "m4v", "mkv", "mov", "mp4", "mpe",
            "mpeg", "mpg", "mts", "mxf", "ts", "vob", "webm", "wmv",
        ];
        for ext in SPEC_EXTENSIONS {
            let p = std::path::PathBuf::from(format!("a.{}", ext));
            let mime = mime_for_path(&p);
            assert_ne!(
                mime, "application/octet-stream",
                "extension `.{}` falls through to octet-stream",
                ext
            );
        }
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

    #[test]
    fn test_library_album_deserializes_from_immich_shape() {
        let album: LibraryAlbum = serde_json::from_value(serde_json::json!({
            "id": "album-1",
            "albumName": "Trips",
            "assetCount": 42,
            "albumThumbnailAssetId": "asset-9",
            "createdAt": "2024-01-01T00:00:00.000Z",
            "updatedAt": "2024-01-02T00:00:00.000Z",
            "description": "Vacation"
        }))
        .unwrap();

        assert_eq!(album.id, "album-1");
        assert_eq!(album.album_name, "Trips");
        assert_eq!(album.asset_count, 42);
        assert_eq!(album.thumbnail_asset_id.as_deref(), Some("asset-9"));
        assert_eq!(album.description, "Vacation");
    }

    #[test]
    fn test_library_asset_deserializes_from_search_result_shape() {
        let asset: LibraryAsset = serde_json::from_value(serde_json::json!({
            "id": "asset-1",
            "originalFileName": "IMG_0001.JPG",
            "originalMimeType": "image/jpeg",
            "fileCreatedAt": "2024-01-01T12:00:00.000Z",
            "type": "IMAGE",
            "thumbhash": "abcd",
            "width": 4032.0,
            "height": 3024.0
        }))
        .unwrap();

        assert_eq!(asset.id, "asset-1");
        assert_eq!(asset.filename, "IMG_0001.JPG");
        assert_eq!(asset.mime_type, "image/jpeg");
        assert_eq!(asset.asset_type, "IMAGE");
        assert_eq!(asset.thumbhash.as_deref(), Some("abcd"));
        assert_eq!(asset.width, Some(4032.0));
        assert_eq!(asset.height, Some(3024.0));
        assert!(asset.checksum.is_none());
    }

    #[test]
    fn test_search_response_deserializes_items() {
        let response: SearchResponse = serde_json::from_value(serde_json::json!({
            "assets": {
                "items": [
                    {
                        "id": "asset-1",
                        "originalFileName": "IMG_0001.JPG",
                        "originalMimeType": "image/jpeg",
                        "fileCreatedAt": "2024-01-01T12:00:00.000Z",
                        "type": "IMAGE",
                        "thumbhash": null,
                        "width": 12.0,
                        "height": 10.0
                    }
                ]
            }
        }))
        .unwrap();

        assert_eq!(response.assets.items.len(), 1);
        assert_eq!(response.assets.items[0].filename, "IMG_0001.JPG");
        assert!(response.assets.next_page.is_none());
    }

    #[test]
    fn test_search_response_parses_next_page() {
        let response: SearchResponse = serde_json::from_value(serde_json::json!({
            "assets": {
                "items": [],
                "nextPage": "2"
            }
        }))
        .unwrap();
        assert_eq!(response.assets.next_page.as_deref(), Some("2"));
    }

    #[test]
    fn test_server_structs_deserialize() {
        let stats: ServerStats = serde_json::from_value(serde_json::json!({
            "images": 100,
            "videos": 25,
            "total": 125
        }))
        .unwrap();
        let about: ServerAbout = serde_json::from_value(serde_json::json!({
            "version": "1.132.0"
        }))
        .unwrap();

        assert_eq!(stats.total, 125);
        assert_eq!(about.version, "1.132.0");
    }

    #[test]
    fn test_thumbnail_size_serialization_values() {
        assert_eq!(ThumbnailSize::Thumbnail.as_str(), "thumbnail");
        assert_eq!(ThumbnailSize::Preview.as_str(), "preview");
    }

    #[test]
    fn test_normalize_file_timestamps_prefers_earliest_available_time_for_created_at() {
        let (created, modified) =
            normalize_file_timestamps(Some(1_704_153_600), Some(1_704_067_200), 99);

        assert_eq!(created, 1_704_067_200);
        assert_eq!(modified, 1_704_067_200);
    }

    #[test]
    fn test_normalize_file_timestamps_falls_back_to_created_time_when_modified_is_missing() {
        let (created, modified) = normalize_file_timestamps(Some(1_704_067_200), None, 99);

        assert_eq!(created, 1_704_067_200);
        assert_eq!(modified, 1_704_067_200);
    }

    #[test]
    fn test_looks_like_iana_timezone() {
        assert!(looks_like_iana_timezone("Asia/Kolkata"));
        assert!(looks_like_iana_timezone("America/New_York"));
        assert!(!looks_like_iana_timezone("UTC"));
        assert!(!looks_like_iana_timezone("Africa Abidjan"));
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

    #[test]
    fn person_deserializes_with_is_hidden_field() {
        let person: Person = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "Alice",
            "isHidden": true
        }))
        .expect("person json");
        assert_eq!(person.id, "p1");
        assert_eq!(person.name, "Alice");
        assert!(person.is_hidden);
    }

    #[test]
    fn person_defaults_is_hidden_when_missing() {
        let person: Person = serde_json::from_value(serde_json::json!({
            "id": "p2",
            "name": ""
        }))
        .expect("person json");
        assert!(!person.is_hidden, "missing isHidden must default to false");
    }

    #[test]
    fn server_statistics_parses_usage_by_user() {
        let stats: ServerStatistics = serde_json::from_value(serde_json::json!({
            "photos": 42,
            "videos": 7,
            "usage": 1024,
            "usageByUser": [
                {
                    "userId": "u1",
                    "userName": "alice",
                    "photos": 10,
                    "videos": 2,
                    "usage": 512,
                    "quotaSizeInBytes": 4096
                }
            ]
        }))
        .expect("server statistics json");
        assert_eq!(stats.photos, 42);
        assert_eq!(stats.videos, 7);
        assert_eq!(stats.usage, 1024);
        assert_eq!(stats.usage_by_user.len(), 1);
        let user = &stats.usage_by_user[0];
        assert_eq!(user.user_name, "alice");
        assert_eq!(user.photos, 10);
        assert_eq!(user.quota_size_in_bytes, Some(4096));
    }

    #[test]
    fn server_statistics_tolerates_missing_quota() {
        let stats: ServerStatistics = serde_json::from_value(serde_json::json!({
            "photos": 0,
            "videos": 0,
            "usage": 0,
            "usageByUser": [
                { "userId": "u1", "userName": "anon", "photos": 0, "videos": 0, "usage": 0 }
            ]
        }))
        .expect("server statistics json");
        assert_eq!(stats.usage_by_user[0].quota_size_in_bytes, None);
    }
}
