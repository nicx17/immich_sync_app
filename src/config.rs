//! Handles persistent configuration loading and provides desktop keyring access helpers.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Defines per-folder filters and guardrails applied before a file is queued for upload.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderRules {
    #[serde(default)]
    pub ignore_hidden: bool,
    #[serde(default)]
    pub max_file_size_mb: Option<u64>,
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
}

impl FolderRules {
    pub fn normalized_extensions(&self) -> Vec<String> {
        self.allowed_extensions
            .iter()
            .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|ext| !ext.is_empty())
            .collect()
    }

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
}

/// A watch path entry stored in config.
///
/// Older configs may contain plain strings, while newer entries can also store per-folder
/// album targeting metadata.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum WatchPathEntry {
    Simple(String),
    WithConfig {
        path: String,
        #[serde(default)]
        album_id: Option<String>,
        #[serde(default)]
        album_name: Option<String>,
        #[serde(default)]
        rules: FolderRules,
    },
}

impl WatchPathEntry {
    pub fn path(&self) -> &str {
        match self {
            WatchPathEntry::Simple(p) => p,
            WatchPathEntry::WithConfig { path, .. } => path,
        }
    }
    pub fn album_name(&self) -> Option<&str> {
        match self {
            WatchPathEntry::Simple(_) => None,
            WatchPathEntry::WithConfig { album_name, .. } => album_name.as_deref(),
        }
    }

    pub fn rules(&self) -> FolderRules {
        match self {
            WatchPathEntry::Simple(_) => FolderRules::default(),
            WatchPathEntry::WithConfig { rules, .. } => rules.clone(),
        }
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

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub enum StartupCatchupMode {
    #[default]
    Full,
    RecentOnly,
    NewFilesOnly,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigData {
    #[serde(default)]
    pub internal_url: String,
    #[serde(default)]
    pub external_url: String,
    #[serde(default = "default_true")]
    pub internal_url_enabled: bool,
    #[serde(default = "default_true")]
    pub external_url_enabled: bool,
    #[serde(default)]
    pub watch_paths: Vec<WatchPathEntry>,
    #[serde(default)]
    pub run_on_startup: bool,
    #[serde(default)]
    pub album_sync: bool,
    #[serde(default)]
    pub delete_after_sync: bool,
    #[serde(default)]
    pub pause_on_metered_network: bool,
    #[serde(default)]
    pub pause_on_battery_power: bool,
    /// Whether automatic background monitoring/upload discovery is enabled.
    #[serde(default = "default_true")]
    pub background_sync_enabled: bool,
    /// Whether desktop notifications (sync summary, connectivity lost, etc.) are shown.
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    #[serde(default)]
    pub startup_catchup_mode: StartupCatchupMode,
    /// Number of parallel upload workers (1–10). Defaults to 3.
    #[serde(default = "default_upload_concurrency")]
    pub upload_concurrency: u8,
    /// Quiet-hours window start (local clock hour, 0–23). `None` means disabled.
    #[serde(default)]
    pub quiet_hours_start: Option<u8>,
    /// Quiet-hours window end (local clock hour, 0–23, exclusive).
    #[serde(default)]
    pub quiet_hours_end: Option<u8>,
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
            album_sync: false,
            delete_after_sync: false,
            pause_on_metered_network: false,
            pause_on_battery_power: false,
            background_sync_enabled: true,
            notifications_enabled: true,
            startup_catchup_mode: StartupCatchupMode::default(),
            upload_concurrency: default_upload_concurrency(),
            quiet_hours_start: None,
            quiet_hours_end: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_upload_concurrency() -> u8 {
    3
}

pub struct Config {
    pub data: ConfigData,
    pub config_file: PathBuf,
}

impl Config {
    /// Load the config from the standard Mimick config path, creating a default file if missing.
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("mimick");

        let config_file = config_dir.join("config.json");

        let mut config = Config {
            data: ConfigData::default(),
            config_file,
        };

        config.load();
        config
    }

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

    pub fn save(&self) -> bool {
        if let Some(parent) = self.config_file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(content) = serde_json::to_string_pretty(&self.data) {
            let ok = fs::write(&self.config_file, content).is_ok();
            if ok {
                log::info!("Config saved to: {}", self.config_file.display());
            } else {
                log::error!("Failed to write config: {}", self.config_file.display());
            }
            ok
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
                let attributes = vec![("service", "mimick"), ("account", "api_key")];
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
                let attributes = vec![("service", "mimick"), ("account", "api_key")];
                keyring
                    .create_item("Mimick API Key", &attributes, secret.as_bytes(), true)
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
        assert!(data.background_sync_enabled);
        assert!(!data.album_sync); // default is false for bool
    }

    #[test]
    fn test_config_data_background_sync_defaults_true_when_missing() {
        let data: ConfigData = serde_json::from_str("{}").unwrap();
        assert!(data.background_sync_enabled);
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
        };

        assert!(rules.matches(&path));

        let restricted = FolderRules {
            ignore_hidden: false,
            max_file_size_mb: Some(0),
            allowed_extensions: vec!["png".into()],
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
}
