//! Provides live filesystem monitoring, file-settling checks, and checksum generation.
//!
//! Uses `notify` watchers (inotify on Linux, polling under Flatpak) to
//! detect new or modified files in watch-path directories. Files are
//! settled via a debounce window before being queued, and checksums are
//! computed using BLAKE3 with chunked streaming to avoid large allocations.

use crate::config::{FolderSyncMethod, WatchPathEntry, best_matching_watch_entry};
use crate::media_kinds;
use crate::watch_path_display::display_watch_path;
use notify::{Config as NotifyConfig, EventKind, PollWatcher, RecursiveMode, Watcher};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Number of consecutive stable size checks required before a file is considered complete.
const REQUIRED_STABLE_COUNTS: u32 = 3;
const CHECK_INTERVAL_MS: u64 = 1000;
const IDLE_TIMEOUT_SECS: u64 = 300;
const FLATPAK_POLL_INTERVAL_MS: u64 = 2000;

/// File monitor that coordinates watching directories and reporting events.
pub struct Monitor {
    /// Watch folders entry list configuration.
    watch_paths: Vec<WatchPathEntry>,
    /// State flag whether background sync is active.
    background_sync_enabled: bool,
}

#[derive(Debug, Clone)]
pub enum MonitorEvent {
    /// Emitted when a file is fully populated and checksummed.
    Ready {
        /// Absolute path of the stabilized file.
        path: String,
        /// File SHA-1 checksum.
        checksum: String,
        /// Companion XMP sidecar path, if one was found on disk.
        sidecar_path: Option<String>,
    },
    /// Emitted when a watched file is deleted.
    Deleted {
        /// Absolute path of the deleted file.
        path: String,
    },
}

/// Monitor commands passed down the main event loop.
enum MonitorCommand {
    /// Replace the live watch folders and settings configuration.
    ReplaceWatchPaths {
        /// Updated watch folder entry list.
        watch_paths: Vec<WatchPathEntry>,
        /// State flag whether background sync is active.
        background_sync_enabled: bool,
    },
}

#[derive(Clone)]
pub struct MonitorHandle {
    /// Command sender channel pointing at the main loop.
    command_tx: std::sync::mpsc::Sender<MonitorCommand>,
}

impl Monitor {
    /// Initialize a new file Monitor.
    pub fn new(watch_paths: Vec<WatchPathEntry>, background_sync_enabled: bool) -> Self {
        Self {
            watch_paths,
            background_sync_enabled,
        }
    }

    /// Start the watcher thread and emit file-ready and deletion events.
    ///
    /// Hashing is offloaded to a bounded worker pool (N = num_cpus / 2, capped at 4)
    /// so bursts of file events don't serialise on hash latency.
    pub fn start(&self, tx: mpsc::Sender<MonitorEvent>) -> MonitorHandle {
        let watch_paths = self.watch_paths.clone();
        let background_sync_enabled = self.background_sync_enabled;
        let handle = tokio::runtime::Handle::current();
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        let worker_count = (num_cpus::get() / 2).clamp(1, 4);
        let (work_tx, work_rx) = tokio::sync::mpsc::channel::<String>(32);

        let active_tasks: std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>> =
            std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new()));

        spawn_hash_workers(
            &handle,
            worker_count,
            work_rx,
            tx.clone(),
            active_tasks.clone(),
        );
        log::info!("Started {} hash worker(s) for file monitor", worker_count);

        std::thread::spawn(move || {
            let (notify_tx, notify_rx) = std::sync::mpsc::channel();
            let mut watcher = match create_watcher(notify_tx) {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Failed to create file watcher: {:?}", e);
                    return;
                }
            };

            let mut watch_paths = watch_paths;
            let mut background_sync_enabled = background_sync_enabled;
            let mut watched_roots = Vec::<PathBuf>::new();
            replace_watches(
                &mut *watcher,
                &mut watched_roots,
                &watch_paths,
                background_sync_enabled,
            );

            let mut debounce_map: HashMap<String, Instant> = HashMap::new();

            loop {
                while let Ok(command) = command_rx.try_recv() {
                    match command {
                        MonitorCommand::ReplaceWatchPaths {
                            watch_paths: new_paths,
                            background_sync_enabled: new_background_sync_enabled,
                        } => {
                            watch_paths = new_paths;
                            background_sync_enabled = new_background_sync_enabled;
                            replace_watches(
                                &mut *watcher,
                                &mut watched_roots,
                                &watch_paths,
                                background_sync_enabled,
                            );
                        }
                    }
                }

                let res = match notify_rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(res) => res,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };

                match res {
                    Ok(event) => handle_notify_event(
                        event,
                        &watch_paths,
                        &tx,
                        &work_tx,
                        &active_tasks,
                        &mut debounce_map,
                    ),
                    Err(e) => log::error!("Watch error: {:?}", e),
                }
            }

            log::warn!("File watcher thread exiting.");
        });

        MonitorHandle { command_tx }
    }
}

fn spawn_hash_workers(
    handle: &tokio::runtime::Handle,
    worker_count: usize,
    work_rx: tokio::sync::mpsc::Receiver<String>,
    tx_out: mpsc::Sender<MonitorEvent>,
    active_tasks: std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
) {
    let work_rx = std::sync::Arc::new(tokio::sync::Mutex::new(work_rx));
    for i in 0..worker_count {
        let rx = work_rx.clone();
        let tx_out = tx_out.clone();
        let active = active_tasks.clone();
        handle.spawn(async move {
            loop {
                let path_str = {
                    let mut guard = rx.lock().await;
                    match guard.recv().await {
                        Some(p) => p,
                        None => break,
                    }
                };

                if wait_for_file_completion(&path_str).await {
                    hash_and_emit(&path_str, &tx_out).await;
                } else {
                    log::warn!("File never stabilised, skipping: {}", path_str);
                }

                active.lock().remove(&path_str);
            }
            log::debug!("Hash worker {} exiting", i);
        });
    }
}

async fn hash_and_emit(path_str: &str, tx_out: &mpsc::Sender<MonitorEvent>) {
    let p_clone = path_str.to_owned();
    match tokio::task::spawn_blocking(move || compute_sha1_chunked(&p_clone)).await {
        Ok(Ok(checksum)) => {
            let sidecar_path = crate::sidecar::find_sidecar(std::path::Path::new(path_str))
                .map(|p| p.to_string_lossy().into_owned());
            log::info!("File ready: {} (sha1={})", path_str, checksum);
            let _ = tx_out
                .send(MonitorEvent::Ready {
                    path: path_str.to_owned(),
                    checksum,
                    sidecar_path,
                })
                .await;
        }
        Ok(Err(e)) => {
            log::error!("Checksum error for {}: {}", path_str, e);
        }
        Err(e) => {
            log::error!("Checksum task panicked for {}: {}", path_str, e);
        }
    }
}

fn handle_notify_event(
    event: notify::Event,
    watch_paths: &[WatchPathEntry],
    tx: &mpsc::Sender<MonitorEvent>,
    work_tx: &tokio::sync::mpsc::Sender<String>,
    active_tasks: &std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
    debounce_map: &mut HashMap<String, Instant>,
) {
    let Some(kind) = monitor_event_kind(&event.kind) else {
        return;
    };

    for path in event.paths {
        if should_skip_path(&path, kind) {
            continue;
        }

        let path_str = path.to_string_lossy().into_owned();
        let Some(matched_entry) = best_matching_watch_entry(&path, watch_paths) else {
            continue;
        };
        let rules = matched_entry.rules();

        match kind {
            MonitorFsEvent::Delete => process_delete_path(&path, path_str, &rules, tx),
            MonitorFsEvent::Upsert => {
                process_upsert_path(&path, path_str, &rules, active_tasks, debounce_map, work_tx)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MonitorFsEvent {
    Upsert,
    Delete,
}

fn monitor_event_kind(kind: &EventKind) -> Option<MonitorFsEvent> {
    if matches!(kind, EventKind::Create(_) | EventKind::Modify(_)) {
        Some(MonitorFsEvent::Upsert)
    } else if matches!(kind, EventKind::Remove(_)) {
        Some(MonitorFsEvent::Delete)
    } else {
        None
    }
}

fn should_skip_path(path: &std::path::Path, kind: MonitorFsEvent) -> bool {
    (kind == MonitorFsEvent::Upsert && path.is_dir()) || !is_supported_media_path(path)
}

fn process_delete_path(
    path: &std::path::Path,
    path_str: String,
    rules: &crate::config::FolderRules,
    tx: &mpsc::Sender<MonitorEvent>,
) {
    if rules.delete_folder_to_album && !is_temporary_file(path) {
        log::info!("Deleted file event: {}", path_str);
        let _ = tx.blocking_send(MonitorEvent::Deleted { path: path_str });
    }
}

fn process_upsert_path(
    path: &Path,
    path_str: String,
    rules: &crate::config::FolderRules,
    active_tasks: &std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
    debounce_map: &mut HashMap<String, Instant>,
    work_tx: &tokio::sync::mpsc::Sender<String>,
) {
    if rules.sync_method == FolderSyncMethod::DownloadOnly
        || is_temporary_file(path)
        || !rules.matches(path)
    {
        return;
    }

    if active_tasks.lock().contains(&path_str) {
        return;
    }

    let now = Instant::now();
    let debounce_ok = debounce_map
        .get(&path_str)
        .map(|last| now.duration_since(*last) > Duration::from_secs(2))
        .unwrap_or(true);

    if !debounce_ok {
        return;
    }

    if debounce_map.len() > 1000 {
        let cutoff = now - Duration::from_secs(60);
        debounce_map.retain(|_, last| *last > cutoff);
    }

    log::info!("New file event: {}", path_str);
    debounce_map.insert(path_str.clone(), now);
    active_tasks.lock().insert(path_str.clone());

    if let Err(err) = work_tx.blocking_send(path_str) {
        log::warn!("Failed to send work to hash pool: {}", err);
    }
}

impl MonitorHandle {
    pub fn replace_watch_paths(
        &self,
        watch_paths: Vec<WatchPathEntry>,
        background_sync_enabled: bool,
    ) {
        if let Err(err) = self.command_tx.send(MonitorCommand::ReplaceWatchPaths {
            watch_paths,
            background_sync_enabled,
        }) {
            log::warn!("Could not update watch paths on the live monitor: {}", err);
        }
    }
}

/// Replace live directories being monitored by the given watcher.
fn replace_watches(
    watcher: &mut dyn Watcher,
    watched_roots: &mut Vec<PathBuf>,
    watch_paths: &[WatchPathEntry],
    background_sync_enabled: bool,
) {
    for path in watched_roots.drain(..) {
        if let Err(err) = watcher.unwatch(&path) {
            log::debug!("Could not unwatch '{}': {:?}", path.display(), err);
        }
    }

    let mut any_watching = false;
    for entry in watch_paths {
        let p = Path::new(entry.path());
        if p.exists() {
            match watcher.watch(p, RecursiveMode::Recursive) {
                Ok(_) => {
                    log::info!("Watching: {}", display_watch_path(entry.path()));
                    watched_roots.push(p.to_path_buf());
                    any_watching = true;
                }
                Err(e) => log::warn!("Failed to watch '{}': {:?}", entry.path(), e),
            }
        } else {
            log::warn!("Watch path does not exist, skipping: {}", entry.path());
        }
    }

    if !any_watching {
        if background_sync_enabled {
            log::warn!("No valid watch paths. File monitoring is idle until a folder is added.");
        } else {
            log::info!("Background sync disabled. File monitoring is idle until it is enabled.");
        }
    }
}

/// Create recommended watcher fallback or poll watcher depending on flatpak containerisation.
fn create_watcher(
    notify_tx: std::sync::mpsc::Sender<notify::Result<notify::Event>>,
) -> notify::Result<Box<dyn Watcher>> {
    if is_flatpak_sandbox() {
        log::info!(
            "Using polling file watcher in Flatpak for portal-selected folders ({}ms interval)",
            FLATPAK_POLL_INTERVAL_MS
        );
        let config = NotifyConfig::default()
            .with_poll_interval(Duration::from_millis(FLATPAK_POLL_INTERVAL_MS));
        Ok(Box::new(PollWatcher::new(notify_tx, config)?))
    } else {
        Ok(Box::new(notify::recommended_watcher(notify_tx)?))
    }
}

/// Check if running inside Flatpak sandbox environment.
fn is_flatpak_sandbox() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Wait for a file's size to stop changing before treating it as upload-ready.
async fn wait_for_file_completion(path: &str) -> bool {
    let mut last_size: i64 = -1;
    let mut stable_count: u32 = 0;
    let mut last_change = Instant::now();

    loop {
        if last_change.elapsed().as_secs() >= IDLE_TIMEOUT_SECS {
            log::warn!(
                "Timeout: file stayed inactive for {}s: {}",
                IDLE_TIMEOUT_SECS,
                path
            );
            return false;
        }

        match tokio::fs::metadata(path).await {
            Ok(meta) => {
                let size = meta.len() as i64;
                if size == last_size && size > 0 {
                    stable_count += 1;
                    last_change = Instant::now();
                    if stable_count >= REQUIRED_STABLE_COUNTS {
                        return true;
                    }
                } else {
                    if size != last_size {
                        last_change = Instant::now(); // file is still growing
                    }
                    stable_count = 0;
                    last_size = size;
                }
            }
            Err(_) => return false,
        }

        tokio::time::sleep(Duration::from_millis(CHECK_INTERVAL_MS)).await;
    }
}

/// Compute SHA-1 in chunks so large media files never need to be read fully into memory.
pub(crate) fn compute_sha1_chunked(path: &str) -> io::Result<String> {
    const BUF_SIZE: usize = 65536;
    let file = fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(BUF_SIZE, file);
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Return whether a path points to a supported media file rather than a directory.
pub(crate) fn is_supported_media_path(path: &Path) -> bool {
    if media_kinds::is_supported_path(path) {
        return true;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    media_kinds::is_supported_ext(ext.as_deref().unwrap_or(""))
}

pub(crate) fn is_temporary_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            let name = name.to_ascii_lowercase();
            name.ends_with(".tmp")
                || name.ends_with(".part")
                || name.ends_with(".crdownload")
                || name.ends_with('~')
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        compute_sha1_chunked, is_flatpak_sandbox, is_supported_media_path, is_temporary_file,
    };
    use std::io::{BufReader, Read, Write};
    use std::path::Path;
    use tempfile::NamedTempFile;

    /// Compute BLAKE3 in chunks. Prepared for Phase 1.5 when hashing switches
    /// from SHA-1 to BLAKE3 for local content identity. Will be promoted to
    /// `pub(crate)` once it replaces SHA-1 in the production path.
    fn compute_blake3_chunked(path: &str) -> std::io::Result<String> {
        const BUF_SIZE: usize = 65536;
        let file = std::fs::File::open(path)?;
        let mut reader = BufReader::with_capacity(BUF_SIZE, file);
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; BUF_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    #[test]
    fn test_compute_sha1_chunked() {
        let mut file = NamedTempFile::new().unwrap();
        // SHA1 of "hello world" is 2aae6c35c94fcfb415dbe95f408b9ce91ee846ed
        file.write_all(b"hello world").unwrap();

        let hash = compute_sha1_chunked(file.path().to_str().unwrap()).unwrap();
        assert_eq!(hash, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
    }

    #[test]
    fn test_compute_blake3_chunked() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"hello world").unwrap();

        let hash = compute_blake3_chunked(file.path().to_str().unwrap()).unwrap();
        // Known BLAKE3 hash of "hello world"
        let expected = blake3::hash(b"hello world").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_flatpak_detection_is_false_in_unit_tests() {
        assert!(!is_flatpak_sandbox());
    }

    #[test]
    fn test_temporary_file_detection() {
        assert!(is_temporary_file(Path::new("/tmp/video.mp4.part")));
        assert!(is_temporary_file(Path::new("/tmp/upload.jpg.tmp")));
        assert!(is_temporary_file(Path::new("/tmp/image.png~")));
        assert!(!is_temporary_file(Path::new("/tmp/final.jpg")));
    }

    #[test]
    fn test_supported_media_extensions_include_new_immich_formats() {
        assert!(is_supported_media_path(Path::new("photo.avif")));
        assert!(is_supported_media_path(Path::new("photo.heif")));
        assert!(is_supported_media_path(Path::new("photo.jp2")));
        assert!(is_supported_media_path(Path::new("photo.jxl")));
        assert!(is_supported_media_path(Path::new("photo.psd")));
        assert!(is_supported_media_path(Path::new("photo.svg")));
        assert!(is_supported_media_path(Path::new("video.3gp")));
        assert!(is_supported_media_path(Path::new("video.avi")));
        assert!(is_supported_media_path(Path::new("video.mkv")));
        assert!(is_supported_media_path(Path::new("video.mxf")));
    }
}
