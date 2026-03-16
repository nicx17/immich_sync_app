//! Persistent configuration loading plus desktop keyring access helpers.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
    },
}

#[allow(dead_code)]
impl WatchPathEntry {
    pub fn path(&self) -> &str {
        match self {
            WatchPathEntry::Simple(p) => p,
            WatchPathEntry::WithConfig { path, .. } => path,
        }
    }
    pub fn album_id(&self) -> Option<&str> {
        match self {
            WatchPathEntry::Simple(_) => None,
            WatchPathEntry::WithConfig { album_id, .. } => album_id.as_deref(),
        }
    }
    pub fn album_name(&self) -> Option<&str> {
        match self {
            WatchPathEntry::Simple(_) => None,
            WatchPathEntry::WithConfig { album_name, .. } => album_name.as_deref(),
        }
    }
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
        }
    }
}

fn default_true() -> bool {
    true
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

    /// Look up the API key from the desktop keyring via `secret-tool`.
    pub fn get_api_key(&self) -> Option<String> {
        // Use secret-tool directly to avoid desktop-environment-specific keyring issues.
        match std::process::Command::new("secret-tool")
            .arg("lookup")
            .arg("service")
            .arg("mimick")
            .arg("account")
            .arg("api_key")
            .output()
        {
            Ok(output) if output.status.success() => {
                let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !key.is_empty() {
                    log::debug!("API key retrieved via secret-tool.");
                    return Some(key);
                }
            }
            Ok(out) => log::debug!("secret-tool lookup empty or failed: {:?}", out.status),
            Err(e) => log::debug!("secret-tool not available or failed: {:?}", e),
        }
        None
    }

    /// Store the API key in the desktop keyring via `secret-tool`.
    pub fn set_api_key(&self, key: &str) -> bool {
        use std::io::Write;
        let mut cmd = std::process::Command::new("secret-tool");
        cmd.arg("store")
            .arg("--label=Mimick API Key")
            .arg("service")
            .arg("mimick")
            .arg("account")
            .arg("api_key")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        if let Ok(mut child) = cmd.spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(key.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    log::info!("API key saved via secret-tool.");
                    return true;
                }
            }
        }

        log::error!("Failed to save API key via secret-tool.");
        false
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

    #[test]
    fn test_watch_path_entry_parsing_simple() {
        let json = r#""/home/nick/Pictures""#;
        let entry: WatchPathEntry = serde_json::from_str(json).unwrap();

        assert_eq!(entry.path(), "/home/nick/Pictures");
        assert!(entry.album_id().is_none());
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
        assert_eq!(entry.album_id().unwrap(), "abc-123");
        assert_eq!(entry.album_name().unwrap(), "My Album");
    }

    #[test]
    fn test_config_data_defaults() {
        let data = ConfigData::default();
        assert!(data.internal_url_enabled);
        assert!(data.external_url_enabled);
        assert!(!data.album_sync); // default is false for bool
    }

    #[test]
    fn test_watch_path_strings_helper() {
        let mut data = ConfigData::default();
        data.watch_paths.push(WatchPathEntry::Simple("/a".into()));
        data.watch_paths.push(WatchPathEntry::WithConfig {
            path: "/b".into(),
            album_id: None,
            album_name: None,
        });

        let config = Config {
            data,
            config_file: PathBuf::from("dummy.json"),
        };

        let strings = config.watch_path_strings();
        assert_eq!(strings, vec!["/a".to_string(), "/b".to_string()]);
    }
}
