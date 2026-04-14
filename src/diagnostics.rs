//! Exports a support-friendly diagnostics bundle with sensitive local data redacted for privacy.

use crate::config::Config;
use crate::queue_manager::FileTask;
use crate::state_manager::AppState;
use serde::Serialize;
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
    fs::write(
        bundle_dir.join("privacy-note.txt"),
        "This bundle intentionally omits API keys, raw logs, server URLs, and full local paths.\n",
    )?;

    write_json_pretty(
        &bundle_dir.join("config.redacted.json"),
        &build_config_export(config),
    )?;
    write_json_pretty(
        &bundle_dir.join("status.redacted.json"),
        &build_state_export(state),
    )?;
    write_json_pretty(
        &bundle_dir.join("retries.redacted.json"),
        &build_retry_export(&cache_path(cache_root, "retries.json"))?,
    )?;
    write_json_pretty(
        &bundle_dir.join("synced_index.redacted.json"),
        &build_sync_index_export(&cache_path(cache_root, "synced_index.json"))?,
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
    lines.push(format!(
        "Watched folder count: {}",
        state.watched_folder_count
    ));
    lines.push(format!(
        "Active server route: {}",
        state.active_server_route.as_deref().unwrap_or("none")
    ));
    lines.push(format!("Queue size: {}", state.queue_size));
    lines.push(format!("Processed count: {}", state.processed_count));
    lines.push(format!("Failed count: {}", state.failed_count));
    lines.push(format!(
        "Current file: {}",
        state
            .current_file
            .as_deref()
            .map(redact_path_hint)
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!(
        "Last completed file: {}",
        state
            .last_completed_file
            .as_deref()
            .map(redact_path_hint)
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!(
        "Last error: {}",
        state.last_error.as_deref().unwrap_or("none")
    ));
    lines.push(format!(
        "Suggested fix: {}",
        state.last_error_guidance.as_deref().unwrap_or("none")
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
    lines.push(format!(
        "Notifications enabled: {}",
        config.data.notifications_enabled
    ));
    lines.push(format!(
        "Startup catchup mode: {:?}",
        config.data.startup_catchup_mode
    ));
    lines.push(format!(
        "Upload concurrency: {}",
        config.data.upload_concurrency
    ));
    lines.push(format!(
        "Quiet hours start: {}",
        config
            .data
            .quiet_hours_start
            .map(|h| h.to_string())
            .unwrap_or_else(|| "disabled".to_string())
    ));
    lines.push(format!(
        "Quiet hours end: {}",
        config
            .data
            .quiet_hours_end
            .map(|h| h.to_string())
            .unwrap_or_else(|| "disabled".to_string())
    ));
    lines.push(
        "Sensitive data policy: URLs, API key, logs, and full local paths omitted".to_string(),
    );
    lines.push(String::new());
    lines.push("Recent queue events:".to_string());
    for event in &state.recent_events {
        lines.push(format!(
            "- {} [{}] attempts={} detail={}",
            redact_path_hint(&event.path),
            event.status,
            event.attempts,
            event.detail.as_deref().unwrap_or("none")
        ));
    }

    lines.join("\n")
}

fn cache_path(cache_root: &Path, name: &str) -> PathBuf {
    cache_root.join(name)
}

#[derive(Serialize)]
struct RedactedConfigExport {
    internal_url_enabled: bool,
    external_url_enabled: bool,
    watch_path_count: usize,
    watch_paths_with_custom_album: usize,
    watch_paths_with_rules: usize,
    run_on_startup: bool,
    album_sync: bool,
    delete_after_sync: bool,
    pause_on_metered_network: bool,
    pause_on_battery_power: bool,
    notifications_enabled: bool,
    startup_catchup_mode: String,
    upload_concurrency: u8,
    quiet_hours_start: Option<u8>,
    quiet_hours_end: Option<u8>,
}

#[derive(Serialize)]
struct RedactedQueueEvent {
    path_hint: String,
    status: String,
    detail: Option<String>,
    attempts: u32,
    timestamp: f64,
}

#[derive(Serialize)]
struct RedactedStateExport {
    status: String,
    paused: bool,
    pause_reason: Option<String>,
    watched_folder_count: usize,
    active_server_route: Option<String>,
    queue_size: usize,
    total_queued: usize,
    processed_count: usize,
    failed_count: usize,
    progress: u8,
    last_successful_sync_at: Option<f64>,
    current_file: Option<String>,
    last_completed_file: Option<String>,
    last_error: Option<String>,
    last_error_guidance: Option<String>,
    diagnostics_exports: usize,
    recent_events: Vec<RedactedQueueEvent>,
}

#[derive(Serialize)]
struct RedactedRetryExport {
    total_retry_items: usize,
    reassociate_only_items: usize,
    album_targeted_items: usize,
}

#[derive(Serialize)]
struct RedactedSyncIndexExport {
    total_entries: usize,
    album_named_entries: usize,
    album_id_entries: usize,
}

fn build_config_export(config: &Config) -> RedactedConfigExport {
    let watch_paths_with_custom_album = config
        .data
        .watch_paths
        .iter()
        .filter(|entry| entry.album_name().is_some_and(|name| !name.is_empty()))
        .count();
    let watch_paths_with_rules = config
        .data
        .watch_paths
        .iter()
        .filter(|entry| {
            let rules = entry.rules();
            rules.ignore_hidden
                || rules.max_file_size_mb.is_some()
                || !rules.allowed_extensions.is_empty()
        })
        .count();

    RedactedConfigExport {
        internal_url_enabled: config.data.internal_url_enabled,
        external_url_enabled: config.data.external_url_enabled,
        watch_path_count: config.data.watch_paths.len(),
        watch_paths_with_custom_album,
        watch_paths_with_rules,
        run_on_startup: config.data.run_on_startup,
        album_sync: config.data.album_sync,
        delete_after_sync: config.data.delete_after_sync,
        pause_on_metered_network: config.data.pause_on_metered_network,
        pause_on_battery_power: config.data.pause_on_battery_power,
        notifications_enabled: config.data.notifications_enabled,
        startup_catchup_mode: format!("{:?}", config.data.startup_catchup_mode),
        upload_concurrency: config.data.upload_concurrency,
        quiet_hours_start: config.data.quiet_hours_start,
        quiet_hours_end: config.data.quiet_hours_end,
    }
}

fn build_state_export(state: &AppState) -> RedactedStateExport {
    RedactedStateExport {
        status: state.status.clone(),
        paused: state.paused,
        pause_reason: state.pause_reason.clone(),
        watched_folder_count: state.watched_folder_count,
        active_server_route: state.active_server_route.clone(),
        queue_size: state.queue_size,
        total_queued: state.total_queued,
        processed_count: state.processed_count,
        failed_count: state.failed_count,
        progress: state.progress,
        last_successful_sync_at: state.last_successful_sync_at,
        current_file: state.current_file.as_deref().map(redact_path_hint),
        last_completed_file: state.last_completed_file.as_deref().map(redact_path_hint),
        last_error: state.last_error.clone(),
        last_error_guidance: state.last_error_guidance.clone(),
        diagnostics_exports: state.diagnostics_exports,
        recent_events: state
            .recent_events
            .iter()
            .map(|event| RedactedQueueEvent {
                path_hint: redact_path_hint(&event.path),
                status: event.status.clone(),
                detail: event.detail.clone(),
                attempts: event.attempts,
                timestamp: event.timestamp,
            })
            .collect(),
    }
}

fn build_retry_export(retry_path: &Path) -> io::Result<RedactedRetryExport> {
    let tasks = if retry_path.exists() {
        let content = fs::read_to_string(retry_path)?;
        serde_json::from_str::<Vec<FileTask>>(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(RedactedRetryExport {
        total_retry_items: tasks.len(),
        reassociate_only_items: tasks.iter().filter(|task| task.reassociate_only).count(),
        album_targeted_items: tasks
            .iter()
            .filter(|task| task.album_id.is_some() || task.album_name.is_some())
            .count(),
    })
}

fn build_sync_index_export(sync_index_path: &Path) -> io::Result<RedactedSyncIndexExport> {
    if !sync_index_path.exists() {
        return Ok(RedactedSyncIndexExport {
            total_entries: 0,
            album_named_entries: 0,
            album_id_entries: 0,
        });
    }

    let content = fs::read_to_string(sync_index_path)?;
    let json = serde_json::from_str::<serde_json::Value>(&content).unwrap_or_default();
    let files = json
        .get("files")
        .and_then(|files| files.as_object())
        .cloned()
        .unwrap_or_default();

    let mut album_named_entries = 0usize;
    let mut album_id_entries = 0usize;
    for record in files.values() {
        if record
            .get("album_name")
            .and_then(|value| value.as_str())
            .is_some_and(|name| !name.is_empty())
        {
            album_named_entries += 1;
        }
        if record
            .get("album_id")
            .and_then(|value| value.as_str())
            .is_some_and(|id| !id.is_empty())
        {
            album_id_entries += 1;
        }
    }

    Ok(RedactedSyncIndexExport {
        total_entries: files.len(),
        album_named_entries,
        album_id_entries,
    })
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    fs::write(path, content)
}

fn redact_path_hint(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .unwrap_or_else(|| "[path hidden]".to_string())
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
        assert!(summary.contains("Watched folder count: 0"));
        assert!(summary.contains("Configured watch paths: 1"));
        assert!(
            summary.contains(
                "Sensitive data policy: URLs, API key, logs, and full local paths omitted"
            )
        );
        assert!(summary.contains("a.jpg [failed] attempts=2"));
        assert!(!summary.contains("/photos/a.jpg [failed] attempts=2"));
    }

    #[test]
    fn test_export_bundle_writes_redacted_files_only() {
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
        assert!(bundle_dir.join("privacy-note.txt").exists());
        assert!(bundle_dir.join("config.redacted.json").exists());
        assert!(bundle_dir.join("status.redacted.json").exists());
        assert!(bundle_dir.join("retries.redacted.json").exists());
        assert!(bundle_dir.join("synced_index.redacted.json").exists());
        assert!(!bundle_dir.join("config.json").exists());
        assert!(!bundle_dir.join("status.json").exists());
        assert!(!bundle_dir.join("retries.json").exists());
        assert!(!bundle_dir.join("synced_index.json").exists());
        assert!(!bundle_dir.join("mimick.log").exists());

        let config_export = fs::read_to_string(bundle_dir.join("config.redacted.json")).unwrap();
        assert!(config_export.contains("\"watch_path_count\": 0"));
        assert!(!config_export.contains("http://localhost"));

        let retry_export = fs::read_to_string(bundle_dir.join("retries.redacted.json")).unwrap();
        assert!(retry_export.contains("\"total_retry_items\": 0"));
    }
}
