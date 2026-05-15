//! Integrates with the Immich API, handles connectivity failover, and provides album/cache helpers.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiIssue {
    pub summary: String,
    pub guidance: String,
}

#[derive(Debug, Clone)]
pub(super) struct ApiClientSettings {
    pub(super) internal_url: String,
    pub(super) external_url: String,
    pub(super) api_key: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct AlbumSummary {
    pub(super) id: String,
    #[serde(rename = "albumName")]
    pub(super) album_name: String,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct LibraryAlbum {
    pub id: String,
    #[serde(rename = "albumName")]
    pub album_name: String,
    #[serde(rename = "assetCount")]
    pub asset_count: u32,
    #[serde(rename = "albumThumbnailAssetId")]
    pub thumbnail_asset_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "ownerId", default)]
    pub owner_id: String,
    #[serde(default)]
    pub shared: bool,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct LibraryAsset {
    pub id: String,
    #[serde(rename = "originalFileName")]
    pub filename: String,
    #[serde(rename = "originalMimeType")]
    pub mime_type: String,
    #[serde(rename = "fileCreatedAt")]
    pub created_at: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    pub thumbhash: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_checksum_to_hex")]
    pub checksum: Option<String>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSearchFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_name: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lens_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_not_in_album: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_exif: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_deleted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<SortOrder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct ServerStats {
    pub images: u64,
    pub videos: u64,
    pub total: u64,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct ServerAbout {
    pub version: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Person {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct PeopleResponse {
    pub(super) people: Vec<Person>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExploreItem {
    pub value: String,
    pub data: LibraryAsset,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExploreSection {
    #[serde(rename = "fieldName")]
    pub field_name: String,
    pub items: Vec<ExploreItem>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct AssetWithExif {
    pub(super) id: String,
    #[serde(rename = "exifInfo", default)]
    pub(super) exif_info: Option<ExifInfo>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct ExifSearchResponse {
    pub(super) assets: ExifSearchAssets,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct ExifSearchAssets {
    pub(super) items: Vec<AssetWithExif>,
    #[serde(rename = "nextPage", default)]
    pub(super) next_page: Option<String>,
}

/// One city with a representative asset ID for thumbnail display.
pub struct PlaceItem {
    pub city: String,
    pub asset_id: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifInfo {
    #[serde(default)]
    pub make: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub lens_model: Option<String>,
    #[serde(default)]
    pub f_number: Option<f64>,
    #[serde(default)]
    pub focal_length: Option<f64>,
    #[serde(default)]
    pub iso: Option<u32>,
    #[serde(default)]
    pub exposure_time: Option<String>,
    #[serde(default)]
    pub file_size_in_byte: Option<u64>,
    #[serde(default)]
    pub date_time_original: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub exif_image_width: Option<u32>,
    #[serde(default)]
    pub exif_image_height: Option<u32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDetails {
    #[serde(default)]
    pub exif_info: Option<ExifInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailSize {
    Thumbnail,
    Preview,
}

impl ThumbnailSize {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            ThumbnailSize::Thumbnail => "thumbnail",
            ThumbnailSize::Preview => "preview",
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct SearchResponse {
    pub(super) assets: SearchAssetSection,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct SearchAssetSection {
    pub(super) items: Vec<LibraryAsset>,
    /// Authoritative pagination signal from Immich. Some(page) when more
    /// results exist, None when the search is exhausted. We only need its
    /// presence — has_more is computed as `next_page.is_some()`.
    #[serde(rename = "nextPage", default)]
    pub(super) next_page: Option<String>,
}

pub struct ImmichApiClient {
    pub client: Client,
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
    albums_fetched: Mutex<bool>,
    thumbnail_semaphore: Arc<Semaphore>,
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

    pub async fn active_route_label(&self) -> Option<String> {
        let active = self.active_url.lock().await.clone()?;
        Some(self.route_label_for_url(&active))
    }

    pub async fn latest_issue(&self) -> Option<ApiIssue> {
        self.last_issue.lock().await.clone()
    }

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

    pub(super) async fn set_issue(&self, issue: ApiIssue) {
        *self.last_issue.lock().await = Some(issue);
    }

    pub(super) async fn clear_issue(&self) {
        *self.last_issue.lock().await = None;
    }

    pub(super) fn settings_snapshot(&self) -> ApiClientSettings {
        self.settings.read().clone()
    }

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
            "heic", "heif", "hif", "insp", "jp2", "jpe", "jxl", "mpo", "svg", "tif", "tiff",
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
}
