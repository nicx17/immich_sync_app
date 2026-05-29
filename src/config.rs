//! Handles persistent configuration loading and provides desktop keyring access helpers.
//!
//! Configuration lives in a JSON file under the XDG config directory.
//! Watch-path entries support both simple paths and extended per-folder
//! rules (sync method, extension filters, size limits). The API key is
//! stored in the desktop keyring via `oo7` and never written to disk.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Defines per-folder filters and guardrails applied before a file is queued for upload.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderRules {
    /// True if hidden files/folders should be ignored during sync.
    #[serde(default)]
    pub ignore_hidden: bool,
    /// Maximum file size allowed for upload, in Megabytes.
    #[serde(default)]
    pub max_file_size_mb: Option<u64>,
    /// List of file extensions permitted for sync.
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
    /// Selected synchronization direction method.
    #[serde(default)]
    pub sync_method: FolderSyncMethod,
    /// Override startup catch-up mode, or None to use global setting.
    #[serde(default)]
    pub startup_catchup_mode: Option<StartupCatchupMode>,
    /// Delete local file when its corresponding remote asset is removed.
    #[serde(default)]
    pub delete_folder_to_album: bool,
    /// Delete remote asset when its corresponding local file is removed.
    #[serde(default)]
    pub delete_album_to_folder: bool,
    /// Per-folder override for XMP sidecar upload. `None` inherits the global
    /// `upload_xmp_sidecars` setting; `Some(true/false)` overrides it.
    #[serde(default)]
    pub include_xmp_sidecar: Option<bool>,
}

impl FolderRules {
    /// Return the list of allowed extensions trimmed and lowercased.
    pub fn normalized_extensions(&self) -> Vec<String> {
        self.allowed_extensions
            .iter()
            .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|ext| !ext.is_empty())
            .collect()
    }

    /// Check if a specific file path meets all active folder validation rules.
    pub fn matches(&self, path: &Path) -> bool {
        if self.ignore_hidden
            && path.components().any(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .map(|part| part.starts_with('.') && part.len() > 1)
                    .unwrap_or(false)
            })
        {
            return false;
        }

        if let Some(limit_mb) = self.max_file_size_mb
            && let Ok(metadata) = std::fs::metadata(path)
            && metadata.len() > limit_mb.saturating_mul(1024 * 1024)
        {
            return false;
        }

        let normalized = self.normalized_extensions();
        if !normalized.is_empty() {
            let ext = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase());
            if ext
                .as_deref()
                .is_none_or(|ext| !normalized.iter().any(|allowed| allowed == ext))
            {
                return false;
            }
        }

        true
    }

    /// Resolve whether XMP sidecar upload is enabled for this folder.
    ///
    /// Returns the per-folder override when set, otherwise falls back to the
    /// caller-supplied global default.
    pub fn xmp_sidecar_enabled(&self, global_default: bool) -> bool {
        self.include_xmp_sidecar.unwrap_or(global_default)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub enum FolderSyncMethod {
    /// Upload folder-only assets and download album-only assets.
    Full,
    /// Only upload assets found in the folder.
    #[default]
    UploadOnly,
    /// Only download assets found in the album.
    DownloadOnly,
}

/// A watch path entry stored in config.
///
/// Older configs may contain plain strings, while newer entries can also store per-folder
/// album targeting metadata.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum WatchPathEntry {
    /// Legacy, simple watched directory path without specific rules or targets.
    Simple(String),
    /// Configured watched directory path containing custom sync targets and filters.
    WithConfig {
        /// Absolute directory path on the local filesystem.
        path: String,
        /// Target Immich album unique identifier.
        #[serde(default)]
        album_id: Option<String>,
        /// Target Immich album name.
        #[serde(default)]
        album_name: Option<String>,
        /// Specific validation and sync rules applied to this path.
        #[serde(default)]
        rules: FolderRules,
    },
}

impl WatchPathEntry {
    /// Retrieve the base path of the watched directory.
    pub fn path(&self) -> &str {
        match self {
            WatchPathEntry::Simple(p) => p,
            WatchPathEntry::WithConfig { path, .. } => path,
        }
    }
    /// Retrieve the target album name, if any.
    pub fn album_name(&self) -> Option<&str> {
        match self {
            WatchPathEntry::Simple(_) => None,
            WatchPathEntry::WithConfig { album_name, .. } => album_name.as_deref(),
        }
    }

    /// Retrieve the folder rules or a default set for simple paths.
    pub fn rules(&self) -> FolderRules {
        match self {
            WatchPathEntry::Simple(_) => FolderRules::default(),
            WatchPathEntry::WithConfig { rules, .. } => rules.clone(),
        }
    }

    /// Retrieve the sync direction configured for this directory.
    pub fn sync_method(&self) -> FolderSyncMethod {
        self.rules().sync_method
    }

    /// Retrieve the catchup strategy override or fallback to global configuration.
    pub fn startup_catchup_mode(&self, fallback: &StartupCatchupMode) -> StartupCatchupMode {
        self.rules()
            .startup_catchup_mode
            .unwrap_or_else(|| fallback.clone())
    }
}

/// Find the most specific configured watch entry that contains `path`.
///
/// Matching is path-aware rather than string-prefix-based so sibling paths like
/// `/home/user/Pictures` and `/home/user/Pictures-backup` are treated correctly.
pub fn best_matching_watch_entry<'a>(
    path: &Path,
    entries: &'a [WatchPathEntry],
) -> Option<&'a WatchPathEntry> {
    entries
        .iter()
        .filter(|entry| path.starts_with(Path::new(entry.path())))
        .max_by_key(|entry| entry.path().len())
}

/// Find a configured watch path entry by its target album name.
pub fn watch_entry_for_album<'a>(
    album_name: &str,
    entries: &'a [WatchPathEntry],
) -> Option<&'a WatchPathEntry> {
    entries
        .iter()
        .find(|entry| entry.album_name() == Some(album_name))
}

/// Catch-up strategy applied to existing files when the application boots.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub enum StartupCatchupMode {
    /// Perform full, deep comparison of all files on the local filesystem.
    #[default]
    Full,
    /// Fast sync covering only files modified or added in the last 24 hours.
    RecentOnly,
    /// Sync only new files added to folders while daemon was offline.
    NewFilesOnly,
}

/// Configuration schema driving application behavior and connection settings.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigData {
    /// Active local Immich server connection address.
    #[serde(default)]
    pub internal_url: String,
    /// Active remote or external connection address.
    #[serde(default)]
    pub external_url: String,
    /// True if internal URL connection is enabled.
    #[serde(default = "default_true")]
    pub internal_url_enabled: bool,
    /// True if external URL connection is enabled.
    #[serde(default = "default_true")]
    pub external_url_enabled: bool,
    /// List of watched local directory entries.
    #[serde(default)]
    pub watch_paths: Vec<WatchPathEntry>,
    /// True if application should launch automatically at user login.
    #[serde(default)]
    pub run_on_startup: bool,
    /// Pause transfers when metered connections are detected.
    #[serde(default)]
    pub pause_on_metered_network: bool,
    /// Pause transfers when system runs on battery.
    #[serde(default)]
    pub pause_on_battery_power: bool,
    /// Whether automatic background monitoring/upload discovery is enabled.
    #[serde(default)]
    pub background_sync_enabled: bool,
    /// Whether desktop notifications (sync summary, connectivity lost, etc.) are shown.
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    /// Default catch-up scanning strategy when background scan starts.
    #[serde(default)]
    pub startup_catchup_mode: StartupCatchupMode,
    /// Number of parallel upload workers (1–10). Defaults to 3.
    #[serde(default = "default_upload_concurrency")]
    pub upload_concurrency: u8,
    /// Quiet-hours window start (local clock hour, 0-23). `None` means disabled.
    #[serde(default)]
    pub quiet_hours_start: Option<u8>,
    /// Quiet-hours window end (local clock hour, 0-23, exclusive).
    #[serde(default)]
    pub quiet_hours_end: Option<u8>,
    /// Whether the built-in library viewer is the primary window.
    #[serde(default)]
    pub library_view_enabled: bool,
    /// Target folder for asset downloads from the library viewer.
    #[serde(default)]
    pub download_target_path: Option<String>,
    /// When true, lightbox loads original full-resolution image instead of preview.
    #[serde(default)]
    pub library_preview_full_resolution: bool,
    /// Grid thumbnail quality: "auto", "thumbnail", "preview". Defaults to auto.
    #[serde(default = "default_grid_quality")]
    pub library_grid_quality: String,
    /// Total on-disk cache cap in megabytes across all subcaches
    /// (thumbnails, raw_decode, exif, video, preview, open-in).
    /// Pruning runs once at startup. Defaults to 2000 MB.
    #[serde(default = "default_cache_disk_cap_mb")]
    pub cache_disk_cap_mb: u32,
    /// When true, decoded RAW textures are cached to disk for faster re-opens.
    /// Disable to save storage; each cached file is a full-resolution PNG.
    #[serde(default)]
    pub raw_decode_cache_enabled: bool,
    /// When true, RAW files are fully demosaiced from sensor data (slow but
    /// highest quality). When false the embedded camera JPEG preview is
    /// extracted instead (near-instant).
    #[serde(default)]
    pub raw_full_decode: bool,
    /// Show people with no assigned name in the Explore view.
    #[serde(default = "default_true")]
    pub show_unnamed_faces: bool,
    /// Include hidden people in the Explore view.
    #[serde(default)]
    pub show_hidden_faces: bool,
    /// Attach XMP sidecar files alongside media during upload. Per-folder
    /// rules can override this global default.
    #[serde(default = "default_true")]
    pub upload_xmp_sidecars: bool,
}

impl Default for ConfigData {
    fn default() -> Self {
        Self {
            internal_url: String::new(),
            external_url: String::new(),
            internal_url_enabled: true,
            external_url_enabled: true,
            watch_paths: Vec::new(),
            run_on_startup: false,
            pause_on_metered_network: false,
            pause_on_battery_power: false,
            background_sync_enabled: false,
            notifications_enabled: true,
            startup_catchup_mode: StartupCatchupMode::default(),
            upload_concurrency: default_upload_concurrency(),
            quiet_hours_start: None,
            quiet_hours_end: None,
            library_view_enabled: false,
            download_target_path: None,
            library_preview_full_resolution: false,
            library_grid_quality: default_grid_quality(),
            cache_disk_cap_mb: default_cache_disk_cap_mb(),
            raw_decode_cache_enabled: false,
            raw_full_decode: false,
            show_unnamed_faces: true,
            show_hidden_faces: false,
            upload_xmp_sidecars: true,
        }
    }
}

/// Helper to default boolean fields to true during serialization.
fn default_true() -> bool {
    true
}

fn default_grid_quality() -> String {
    "auto".to_string()
}

/// Helper to default parallel upload worker threads to 3.
fn default_upload_concurrency() -> u8 {
    3
}

/// Default total on-disk cache cap in MB.
fn default_cache_disk_cap_mb() -> u32 {
    2000
}

/// Persistent config container wrapping loaded schema and file source info.
#[derive(Clone)]
pub struct Config {
    /// Active live configuration switches.
    pub data: ConfigData,
    /// Path to config file source.
    pub config_file: PathBuf,
}

impl Config {
    /// Load the config from the standard Mimick config path, creating a default file if missing.
    pub fn new() -> Self {
        let config_dir = crate::profile::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config").join(crate::profile::dir_segment()));

        let config_file = config_dir.join("config.json");

        let mut config = Config {
            data: ConfigData::default(),
            config_file,
        };

        config.load();
        config
    }

    /// Load or parse configuration state from standard path source.
    pub fn load(&mut self) -> bool {
        if self.config_file.exists() {
            if let Ok(content) = fs::read_to_string(&self.config_file) {
                if let Ok(data) = serde_json::from_str(&content) {
                    self.data = data;
                    log::info!("Config loaded from: {}", self.config_file.display());
                    return true;
                } else {
                    log::warn!("Config parse failed: {}", self.config_file.display());
                }
            }
        } else {
            log::info!(
                "No config found, creating default at: {}",
                self.config_file.display()
            );
            self.save();
        }
        false
    }

    /// Atomically write current configuration values back to disk.
    pub fn save(&self) -> bool {
        if let Ok(content) = serde_json::to_string_pretty(&self.data) {
            match crate::util::atomic_write(&self.config_file, content.as_bytes()) {
                Ok(()) => {
                    log::info!("Config saved to: {}", self.config_file.display());
                    true
                }
                Err(err) => {
                    log::error!(
                        "Failed to write config {}: {}",
                        self.config_file.display(),
                        err
                    );
                    false
                }
            }
        } else {
            false
        }
    }

    /// Look up the API key from the desktop keyring via oo7.
    ///
    /// Automatically selects the correct backend:
    /// - Flatpak sandbox: encrypted file via the Secret portal
    /// - Native: D-Bus Secret Service (GNOME Keyring, KWallet)
    pub fn get_api_key(&self) -> Option<String> {
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let keyring = oo7::Keyring::new().await?;
                let account = crate::profile::keyring_account();
                let attributes: Vec<(&str, &str)> =
                    vec![("service", "mimick"), ("account", account.as_str())];
                let items = keyring.search_items(&attributes).await?;
                if let Some(item) = items.first() {
                    let secret = item.secret().await?;
                    let key = String::from_utf8_lossy(&secret).trim().to_string();
                    if !key.is_empty() {
                        return Ok::<Option<String>, oo7::Error>(Some(key));
                    }
                }
                Ok(None)
            })
        });

        match result {
            Ok(key) => {
                if key.is_some() {
                    log::debug!("API key retrieved via oo7 keyring.");
                }
                key
            }
            Err(e) => {
                log::debug!("oo7 keyring lookup failed: {:?}", e);
                None
            }
        }
    }

    /// Store the API key in the desktop keyring via oo7.
    pub fn set_api_key(&self, key: &str) -> bool {
        let secret = key.to_string();
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let keyring = oo7::Keyring::new().await?;
                let account = crate::profile::keyring_account();
                let attributes: Vec<(&str, &str)> =
                    vec![("service", "mimick"), ("account", account.as_str())];
                let label = match crate::profile::name() {
                    Some(profile) => format!("Mimick API Key ({})", profile),
                    None => "Mimick API Key".to_string(),
                };
                keyring
                    .create_item(&label, &attributes, secret.as_bytes(), true)
                    .await?;
                Ok::<(), oo7::Error>(())
            })
        });

        match result {
            Ok(()) => {
                log::info!("API key saved via oo7 keyring.");
                true
            }
            Err(e) => {
                log::error!("Failed to save API key via oo7 keyring: {:?}", e);
                false
            }
        }
    }

    /// Return all configured watch paths as plain strings for the live monitor.
    pub fn watch_path_strings(&self) -> Vec<String> {
        self.data
            .watch_paths
            .iter()
            .map(|e| e.path().to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_watch_path_entry_parsing_simple() {
        let json = r#""/home/nick/Pictures""#;
        let entry: WatchPathEntry = serde_json::from_str(json).unwrap();

        assert_eq!(entry.path(), "/home/nick/Pictures");
        assert!(matches!(entry, WatchPathEntry::Simple(_)));
    }

    #[test]
    fn test_watch_path_entry_parsing_with_config() {
        let json = r#"{
            "path": "/home/nick/Pictures",
            "album_id": "abc-123",
            "album_name": "My Album"
        }"#;
        let entry: WatchPathEntry = serde_json::from_str(json).unwrap();

        assert_eq!(entry.path(), "/home/nick/Pictures");
        let WatchPathEntry::WithConfig { album_id, .. } = &entry else {
            panic!("expected configured watch entry");
        };
        assert_eq!(album_id.as_deref(), Some("abc-123"));
        assert_eq!(entry.album_name().unwrap(), "My Album");
    }

    #[test]
    fn test_config_data_defaults() {
        let data = ConfigData::default();
        assert!(data.internal_url_enabled);
        assert!(data.external_url_enabled);
        assert!(!data.background_sync_enabled);
    }

    #[test]
    fn test_config_data_background_sync_defaults_false_when_missing() {
        let data: ConfigData = serde_json::from_str("{}").unwrap();
        assert!(!data.background_sync_enabled);
    }

    #[test]
    fn test_legacy_folder_rules_default_to_upload_only_and_global_startup_fallback() {
        let entry: WatchPathEntry = serde_json::from_str(
            r#"{
                "path": "/home/nick/Pictures",
                "rules": {
                    "ignore_hidden": true
                }
            }"#,
        )
        .unwrap();

        let rules = entry.rules();
        assert_eq!(rules.sync_method, FolderSyncMethod::UploadOnly);
        assert_eq!(
            entry.startup_catchup_mode(&StartupCatchupMode::RecentOnly),
            StartupCatchupMode::RecentOnly
        );
        assert!(!rules.delete_folder_to_album);
        assert!(!rules.delete_album_to_folder);
    }

    #[test]
    fn test_watch_path_strings_helper() {
        let mut data = ConfigData::default();
        data.watch_paths.push(WatchPathEntry::Simple("/a".into()));
        data.watch_paths.push(WatchPathEntry::WithConfig {
            path: "/b".into(),
            album_id: None,
            album_name: None,
            rules: FolderRules::default(),
        });

        let config = Config {
            data,
            config_file: PathBuf::from("dummy.json"),
        };

        let strings = config.watch_path_strings();
        assert_eq!(strings, vec!["/a".to_string(), "/b".to_string()]);
    }

    #[test]
    fn test_folder_rules_match_extension_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        fs::write(&path, vec![0u8; 1024]).unwrap();

        let rules = FolderRules {
            ignore_hidden: false,
            max_file_size_mb: Some(1),
            allowed_extensions: vec!["jpg".into(), "png".into()],
            ..FolderRules::default()
        };

        assert!(rules.matches(&path));

        let restricted = FolderRules {
            ignore_hidden: false,
            max_file_size_mb: Some(0),
            allowed_extensions: vec!["png".into()],
            ..FolderRules::default()
        };

        assert!(!restricted.matches(&path));
    }

    #[test]
    fn test_folder_rules_ignore_hidden_path_components() {
        let dir = tempfile::tempdir().unwrap();
        let hidden_dir = dir.path().join(".hidden");
        fs::create_dir_all(&hidden_dir).unwrap();
        let hidden_file = hidden_dir.join("photo.jpg");
        fs::write(&hidden_file, vec![0u8; 16]).unwrap();

        let rules = FolderRules {
            ignore_hidden: true,
            ..FolderRules::default()
        };

        assert!(!rules.matches(&hidden_file));
    }

    #[test]
    fn test_normalized_extensions_trims_and_lowercases() {
        let rules = FolderRules {
            allowed_extensions: vec![" JPG ".into(), ".PNG".into(), "".into()],
            ..FolderRules::default()
        };

        assert_eq!(rules.normalized_extensions(), vec!["jpg", "png"]);
    }

    #[test]
    fn test_best_matching_watch_entry_prefers_most_specific_path() {
        let entries = vec![
            WatchPathEntry::Simple("/home/user/Pictures".into()),
            WatchPathEntry::WithConfig {
                path: "/home/user/Pictures/Trips".into(),
                album_id: Some("album-1".into()),
                album_name: Some("Trips".into()),
                rules: FolderRules::default(),
            },
        ];

        let matched = best_matching_watch_entry(
            Path::new("/home/user/Pictures/Trips/day1/photo.jpg"),
            &entries,
        )
        .unwrap();
        assert_eq!(matched.path(), "/home/user/Pictures/Trips");
        assert_eq!(matched.album_name(), Some("Trips"));
    }

    #[test]
    fn test_best_matching_watch_entry_does_not_match_string_prefix_siblings() {
        let entries = vec![WatchPathEntry::Simple("/home/user/Pictures".into())];

        assert!(
            best_matching_watch_entry(Path::new("/home/user/Pictures-backup/photo.jpg"), &entries)
                .is_none()
        );
    }

    #[test]
    fn face_visibility_defaults_favour_discoverable_named_only() {
        let data = ConfigData::default();
        assert!(
            data.show_unnamed_faces,
            "unnamed people should be visible by default so users see them"
        );
        assert!(
            !data.show_hidden_faces,
            "hidden people stay hidden by default"
        );
    }

    #[test]
    fn face_visibility_flags_round_trip_through_json() {
        let data = ConfigData {
            show_unnamed_faces: false,
            show_hidden_faces: true,
            ..ConfigData::default()
        };
        let json = serde_json::to_string(&data).expect("serialize");
        let restored: ConfigData = serde_json::from_str(&json).expect("deserialize");
        assert!(!restored.show_unnamed_faces);
        assert!(restored.show_hidden_faces);
    }

    #[test]
    fn face_visibility_flags_default_when_absent_in_json() {
        // Older config files written before the flags existed must still load.
        let json = serde_json::to_string(&serde_json::json!({})).unwrap();
        let restored: ConfigData = serde_json::from_str(&json).expect("deserialize legacy config");
        assert!(restored.show_unnamed_faces);
        assert!(!restored.show_hidden_faces);
    }
}
