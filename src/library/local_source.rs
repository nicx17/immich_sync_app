//! Local file enumeration for the library view.
//!
//! Walks the user's currently configured watch paths, applies the same
//! filtering rules used by the sync engine (`FolderRules` + supported-media
//! extensions), and produces a list of `LocalAsset` rows that the library
//! grid can display alongside (or instead of) remote Immich assets.
//!
//! Unlike the sync path, this module does NOT compute checksums on enumeration
//! — that would be too expensive for browsing. Sync state is matched by
//! looking up `SyncIndex.stored_checksum(path)` for paths the engine has
//! already hashed; assets the user hasn't synced yet show as "Local only".

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};

use crate::app_context::AppContext;
use crate::config::WatchPathEntry;
use crate::monitor::{is_supported_media_path, is_temporary_file};

/// A single file enumerated from the user's watched folders.
#[derive(Clone, Debug)]
pub struct LocalAsset {
    /// Absolute path on disk (used as the synthetic row id).
    pub path: PathBuf,
    /// Display name (file_name component).
    pub filename: String,
    /// MIME type, derived from the extension.
    pub mime: String,
    /// "IMAGE" or "VIDEO".
    pub asset_type: &'static str,
    /// File size in bytes (carried for future use — not currently displayed).
    #[allow(dead_code)]
    pub size: u64,
    /// ISO-8601 modification time, used as `created_at` for sort consistency
    /// with remote `LibraryAsset`.
    pub created_at: String,
}

impl LocalAsset {
    /// Synthetic identity used by the GridView model. Using the path keeps
    /// dedup logic stable across enumerations even when modification times
    /// change. Currently unused outside tests, but kept as the canonical
    /// recipe so future call sites match the prefix logic in `mod.rs`.
    #[allow(dead_code)]
    pub fn synthetic_id(&self) -> String {
        format!("local::{}", self.path.display())
    }
}

/// Walk every configured watch path and return matching media files.
///
/// Runs the synchronous walk on a Tokio blocking thread so the UI thread
/// stays responsive even on libraries with tens of thousands of files.
pub async fn enumerate_local(ctx: Arc<AppContext>) -> Vec<LocalAsset> {
    let watch_paths = ctx
        .live_watch_paths
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    tokio::task::spawn_blocking(move || enumerate_blocking(&watch_paths))
        .await
        .unwrap_or_default()
}

fn enumerate_blocking(watch_paths: &[WatchPathEntry]) -> Vec<LocalAsset> {
    let mut out = Vec::new();
    for entry in watch_paths {
        let root = PathBuf::from(entry.path());
        if !root.is_dir() {
            continue;
        }
        let rules = entry.rules();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let read_dir = match std::fs::read_dir(&dir) {
                Ok(iter) => iter,
                Err(_) => continue,
            };
            for child in read_dir.flatten() {
                let path = child.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !is_supported_media_path(&path) || is_temporary_file(&path) {
                    continue;
                }
                if !rules.matches(&path) {
                    continue;
                }
                if let Some(asset) = build_asset(&path) {
                    out.push(asset);
                }
            }
        }
    }
    out
}

fn build_asset(path: &Path) -> Option<LocalAsset> {
    let filename = path.file_name()?.to_string_lossy().into_owned();
    let metadata = std::fs::metadata(path).ok()?;
    let size = metadata.len();
    let created_at = format_modified(&metadata);
    let mime = mime_for_extension(path);
    let asset_type = if mime.starts_with("video/") {
        "VIDEO"
    } else {
        "IMAGE"
    };
    Some(LocalAsset {
        path: path.to_path_buf(),
        filename,
        mime: mime.into(),
        asset_type,
        size,
        created_at,
    })
}

fn mime_for_extension(path: &Path) -> &'static str {
    // Subset of `api_client::mime_for_path` relevant to local browsing.
    // Vendor-specific RAW MIMEs are collapsed to "image/x-raw" because the
    // library view only uses the asset_type bucket; the per-vendor mapping
    // continues to live in api_client for upload paths.
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("avif") => "image/avif",
        Some("bmp") => "image/bmp",
        Some("gif") => "image/gif",
        Some("heic") => "image/heic",
        Some("heif" | "hif") => "image/heif",
        Some("jpe" | "jpeg" | "jpg" | "insp" | "mpo") => "image/jpeg",
        Some("jp2") => "image/jp2",
        Some("jxl") => "image/jxl",
        Some("png") => "image/png",
        Some("psd") => "image/vnd.adobe.photoshop",
        Some("svg") => "image/svg+xml",
        Some("tif" | "tiff") => "image/tiff",
        Some("webp") => "image/webp",
        Some("3gp" | "3gpp") => "video/3gpp",
        Some("avi") => "video/x-msvideo",
        Some("flv") => "video/x-flv",
        Some("insv" | "mp4") => "video/mp4",
        Some("m2t" | "m2ts" | "mts" | "ts") => "video/mp2t",
        Some("m4v") => "video/x-m4v",
        Some("mkv") => "video/x-matroska",
        Some("mpe" | "mpeg" | "mpg") => "video/mpeg",
        Some("mov") => "video/quicktime",
        Some("mxf") => "application/mxf",
        Some("vob") => "video/dvd",
        Some("webm") => "video/webm",
        Some("wmv") => "video/x-ms-wmv",
        Some(_) => "image/x-raw",
        None => "application/octet-stream",
    }
}

fn format_modified(meta: &std::fs::Metadata) -> String {
    let mtime = meta
        .modified()
        .or_else(|_| meta.created())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let datetime: DateTime<Utc> = mtime.into();
    datetime.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Apply a case-insensitive filename substring filter, matching the spec's
/// "Local Search: file name based" requirement.
pub fn filter_by_filename(items: Vec<LocalAsset>, query: &str) -> Vec<LocalAsset> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return items;
    }
    items
        .into_iter()
        .filter(|a| a.filename.to_ascii_lowercase().contains(&needle))
        .collect()
}

/// Decide the sync-state badge for a local asset by checking the SyncIndex.
/// Returns `0` when the file's path is recorded as synced (so the grid will
/// label it "Both"), `1` otherwise (LocalOnly).
pub fn local_sync_state(ctx: &AppContext, path: &Path) -> u32 {
    let Ok(idx) = ctx.sync_index.lock() else {
        return 1;
    };
    if idx.stored_checksum(&path.display().to_string()).is_some() {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(name: &str) -> LocalAsset {
        LocalAsset {
            path: PathBuf::from(name),
            filename: name.into(),
            mime: "image/jpeg".into(),
            asset_type: "IMAGE",
            size: 0,
            created_at: String::new(),
        }
    }

    #[test]
    fn filter_by_filename_is_case_insensitive_substring() {
        let items = vec![make("Beach.JPG"), make("city.jpg"), make("forest.png")];
        let filtered = filter_by_filename(items, "JpG");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_by_filename_empty_query_returns_all() {
        let items = vec![make("a.jpg"), make("b.jpg")];
        assert_eq!(filter_by_filename(items, "  ").len(), 2);
    }

    #[test]
    fn synthetic_id_is_stable_across_clones() {
        let a = make("/tmp/a.jpg");
        assert_eq!(a.synthetic_id(), a.clone().synthetic_id());
    }
}
