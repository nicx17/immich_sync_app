//! Export a support-friendly diagnostics bundle without including secrets.

use crate::config::Config;
use crate::state_manager::AppState;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn export_bundle(destination_root: &Path, state: &AppState) -> io::Result<PathBuf> {
    let config = Config::new();
    let cache_root = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("mimick");
    export_bundle_with_paths(destination_root, state, &config, &cache_root)
}

fn export_bundle_with_paths(
    destination_root: &Path,
    state: &AppState,
    config: &Config,
    cache_root: &Path,
) -> io::Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bundle_dir = destination_root.join(format!("mimick-diagnostics-{}", timestamp));
    fs::create_dir_all(&bundle_dir)?;

    let summary_path = bundle_dir.join("summary.txt");
    fs::write(summary_path, build_summary(config, state))?;

    copy_if_exists(&config.config_file, &bundle_dir.join("config.json"))?;
    copy_if_exists(
        &cache_path(cache_root, "status.json"),
        &bundle_dir.join("status.json"),
    )?;
    copy_if_exists(
        &cache_path(cache_root, "retries.json"),
        &bundle_dir.join("retries.json"),
    )?;
    copy_if_exists(
        &cache_path(cache_root, "synced_index.json"),
        &bundle_dir.join("synced_index.json"),
    )?;
    copy_if_exists(
        &cache_path(cache_root, "mimick.log"),
        &bundle_dir.join("mimick.log"),
    )?;

    Ok(bundle_dir)
}

fn build_summary(config: &Config, state: &AppState) -> String {
    let mut lines = Vec::new();
    lines.push("Mimick diagnostics export".to_string());
    lines.push(format!("Version: {}", env!("CARGO_PKG_VERSION")));
    lines.push(format!("App status: {}", state.status));
    lines.push(format!("Paused: {}", state.paused));
    lines.push(format!(
        "Pause reason: {}",
        state.pause_reason.as_deref().unwrap_or("none")
    ));
    lines.push(format!("Queue size: {}", state.queue_size));
    lines.push(format!("Processed count: {}", state.processed_count));
    lines.push(format!("Failed count: {}", state.failed_count));
    lines.push(format!(
        "Current file: {}",
        state.current_file.as_deref().unwrap_or("none")
    ));
    lines.push(format!(
        "Last completed file: {}",
        state.last_completed_file.as_deref().unwrap_or("none")
    ));
    lines.push(format!(
        "Last error: {}",
        state.last_error.as_deref().unwrap_or("none")
    ));
    lines.push(format!(
        "Configured watch paths: {}",
        config.data.watch_paths.len()
    ));
    lines.push(format!(
        "Pause on metered network: {}",
        config.data.pause_on_metered_network
    ));
    lines.push(format!(
        "Pause on battery power: {}",
        config.data.pause_on_battery_power
    ));
    lines.push("API key: omitted".to_string());
    lines.push(String::new());
    lines.push("Recent queue events:".to_string());
    for event in &state.recent_events {
        lines.push(format!(
            "- {} [{}] attempts={} detail={}",
            event.path,
            event.status,
            event.attempts,
            event.detail.as_deref().unwrap_or("none")
        ));
    }

    lines.join("\n")
}

fn copy_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    if from.exists() {
        fs::copy(from, to)?;
    }
    Ok(())
}

fn cache_path(cache_root: &Path, name: &str) -> PathBuf {
    cache_root.join(name)
}

#[cfg(test)]
mod tests {
    use super::{build_summary, export_bundle_with_paths};
    use crate::config::{Config, ConfigData, WatchPathEntry};
    use crate::state_manager::{AppState, QueueEvent};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_build_summary_contains_recent_events_and_omits_api_key() {
        let config = Config {
            data: ConfigData {
                watch_paths: vec![WatchPathEntry::Simple("/photos".into())],
                pause_on_metered_network: true,
                pause_on_battery_power: false,
                ..ConfigData::default()
            },
            config_file: PathBuf::from("config.json"),
        };
        let mut state = AppState {
            status: "paused".into(),
            paused: true,
            pause_reason: Some("Paused by user".into()),
            ..AppState::default()
        };
        state.recent_events.push(QueueEvent {
            path: "/photos/a.jpg".into(),
            status: "failed".into(),
            detail: Some("Queued for retry".into()),
            attempts: 2,
            timestamp: 1.0,
        });

        let summary = build_summary(&config, &state);
        assert!(summary.contains("App status: paused"));
        assert!(summary.contains("Configured watch paths: 1"));
        assert!(summary.contains("API key: omitted"));
        assert!(summary.contains("/photos/a.jpg [failed] attempts=2"));
    }

    #[test]
    fn test_export_bundle_writes_summary_and_cache_files() {
        let dir = tempdir().unwrap();
        let dest_root = dir.path().join("exports");
        let cache_root = dir.path().join("cache");
        let config_root = dir.path().join("config");
        fs::create_dir_all(&cache_root).unwrap();
        fs::create_dir_all(&config_root).unwrap();

        let config_path = config_root.join("config.json");
        fs::write(&config_path, "{\"internal_url\":\"http://localhost\"}").unwrap();
        fs::write(cache_root.join("status.json"), "{\"status\":\"idle\"}").unwrap();
        fs::write(cache_root.join("retries.json"), "[]").unwrap();
        fs::write(cache_root.join("synced_index.json"), "{\"files\":{}}").unwrap();
        fs::write(cache_root.join("mimick.log"), "hello log").unwrap();

        let config = Config {
            data: ConfigData::default(),
            config_file: config_path,
        };
        let state = AppState::default();

        let bundle_dir =
            export_bundle_with_paths(&dest_root, &state, &config, &cache_root).unwrap();
        assert!(bundle_dir.join("summary.txt").exists());
        assert!(bundle_dir.join("config.json").exists());
        assert!(bundle_dir.join("status.json").exists());
        assert!(bundle_dir.join("retries.json").exists());
        assert!(bundle_dir.join("synced_index.json").exists());
        assert!(bundle_dir.join("mimick.log").exists());
    }
}
