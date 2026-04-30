//! Provides live filesystem monitoring, file-settling checks, and checksum generation for watched paths.

use crate::config::{WatchPathEntry, best_matching_watch_entry};
use crate::watch_path_display::display_watch_path;
use notify::{Config as NotifyConfig, EventKind, PollWatcher, RecursiveMode, Watcher};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// List of allowed media file extensions accepted for upload.
///
/// Sorted alphabetically. Includes still-image formats (incl. RAW),
/// video formats, and high-bit-depth/professional formats supported by Immich.
pub(crate) const MEDIA_EXTENSIONS: &[&str] = &[
    "3fr", "3gp", "3gpp", "ari", "arw", "avi", "avif", "bmp", "cap", "cin", "cr2", "cr3", "crw",
    "dcr", "dng", "erf", "fff", "flv", "gif", "heic", "heif", "hif", "iiq", "insp", "insv", "jp2",
    "jpe", "jpeg", "jpg", "jxl", "k25", "kdc", "m2t", "m2ts", "m4v", "mkv", "mov", "mp4", "mpe",
    "mpeg", "mpg", "mpo", "mrw", "mts", "mxf", "nef", "nrw", "orf", "ori", "pef", "png", "psd",
    "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw", "svg", "tif", "tiff", "ts", "vob", "webm",
    "webp", "wmv", "x3f",
];

/// Number of consecutive stable size checks required before a file is considered complete.
const REQUIRED_STABLE_COUNTS: u32 = 3;
const CHECK_INTERVAL_MS: u64 = 1000;
const IDLE_TIMEOUT_SECS: u64 = 300;
const FLATPAK_POLL_INTERVAL_MS: u64 = 2000;

pub struct Monitor {
    watch_paths: Vec<WatchPathEntry>,
    background_sync_enabled: bool,
}

enum MonitorCommand {
    ReplaceWatchPaths {
        watch_paths: Vec<WatchPathEntry>,
        background_sync_enabled: bool,
    },
}

#[derive(Clone)]
pub struct MonitorHandle {
    command_tx: std::sync::mpsc::Sender<MonitorCommand>,
}

impl Monitor {
    pub fn new(watch_paths: Vec<WatchPathEntry>, background_sync_enabled: bool) -> Self {
        Self {
            watch_paths,
            background_sync_enabled,
        }
    }

    /// Start the watcher thread and emit `(path, sha1_hex)` tuples for ready files.
    pub fn start(&self, tx: mpsc::Sender<(String, String)>) -> MonitorHandle {
        let watch_paths = self.watch_paths.clone();
        let background_sync_enabled = self.background_sync_enabled;
        let handle = tokio::runtime::Handle::current();
        let (command_tx, command_rx) = std::sync::mpsc::channel();

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

            // Debounce map: path -> last seen instant
            let mut debounce_map: HashMap<String, Instant> = HashMap::new();

            // Track files that are currently waiting for size stability.
            // Prevents long-running records (like screencasts) from spawning
            // duplicate tasks every 2 seconds.
            let active_tasks: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
                std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

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
                    Ok(event) => {
                        let is_relevant =
                            matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));

                        if is_relevant {
                            for path in event.paths {
                                if path.is_dir() {
                                    continue;
                                }

                                let ext =
                                    path.extension().map(|e| e.to_string_lossy().to_lowercase());
                                let ext_str = ext.as_deref().unwrap_or("");

                                if !MEDIA_EXTENSIONS.contains(&ext_str) {
                                    continue;
                                }

                                let path_str = path.to_string_lossy().into_owned();
                                if is_temporary_file(&path)
                                    || !best_matching_watch_entry(&path, &watch_paths)
                                        .map(|entry| entry.rules().matches(&path))
                                        .unwrap_or(true)
                                {
                                    continue;
                                }

                                // Bail immediately if we're already waiting on this file to finish.
                                if active_tasks.lock().unwrap().contains(&path_str) {
                                    continue;
                                }

                                let now = Instant::now();
                                let debounce_ok = debounce_map
                                    .get(&path_str)
                                    .map(|last| now.duration_since(*last) > Duration::from_secs(2))
                                    .unwrap_or(true);

                                if !debounce_ok {
                                    continue;
                                }

                                if debounce_map.len() > 1000 {
                                    let cutoff = now - Duration::from_secs(60);
                                    debounce_map.retain(|_, last| *last > cutoff);
                                }

                                log::info!("New file event: {}", path_str);
                                debounce_map.insert(path_str.clone(), now);
                                active_tasks.lock().unwrap().insert(path_str.clone());

                                let tx_clone = tx.clone();
                                let active_clone = active_tasks.clone();
                                handle.spawn(async move {
                                    let is_complete = wait_for_file_completion(&path_str).await;

                                    if is_complete {
                                        let p_clone = path_str.clone();
                                        match tokio::task::spawn_blocking(move || {
                                            compute_sha1_chunked(&p_clone)
                                        })
                                        .await
                                        .unwrap()
                                        {
                                            Ok(checksum) => {
                                                log::info!(
                                                    "File ready: {} (sha1={})",
                                                    path_str,
                                                    checksum
                                                );
                                                let _ = tx_clone
                                                    .send((path_str.clone(), checksum))
                                                    .await;
                                            }
                                            Err(e) => log::error!(
                                                "Checksum error for {}: {}",
                                                path_str,
                                                e
                                            ),
                                        }
                                    } else {
                                        log::warn!("File never stabilised, skipping: {}", path_str);
                                    }

                                    // Clear from active tasks so future modifications can be sensed.
                                    active_clone.lock().unwrap().remove(&path_str);
                                });
                            }
                        }
                    }
                    Err(e) => log::error!("Watch error: {:?}", e),
                }
            }

            log::warn!("File watcher thread exiting.");
        });

        MonitorHandle { command_tx }
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
    if path.is_dir() {
        return false;
    }

    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let ext_str = ext.as_deref().unwrap_or("");
    MEDIA_EXTENSIONS.contains(&ext_str)
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
    use std::io::Write;
    use std::path::Path;
    use tempfile::NamedTempFile;

    #[test]
    fn test_compute_sha1_chunked() {
        let mut file = NamedTempFile::new().unwrap();
        // SHA1 of "hello world" is 2aae6c35c94fcfb415dbe95f408b9ce91ee846ed
        file.write_all(b"hello world").unwrap();

        let hash = compute_sha1_chunked(file.path().to_str().unwrap()).unwrap();
        assert_eq!(hash, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
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
