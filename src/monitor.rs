use std::fs;
use std::io::{self, BufReader, Read};
use std::path::Path;
use sha1::{Sha1, Digest};
use notify::{Watcher, RecursiveMode, EventKind};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Whitelisted media extensions (matches Python ALLOWED_EXTENSIONS)
const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "mp4", "mov", "gif", "webp",
    "tiff", "tif", "raw", "arw", "dng",
];

/// Consecutive stable size checks required before a file is considered complete.
const REQUIRED_STABLE_COUNTS: u32 = 3;
const CHECK_INTERVAL_MS: u64 = 1000;
const IDLE_TIMEOUT_SECS: u64 = 300;

pub struct Monitor {
    watch_paths: Vec<String>,
}

impl Monitor {
    pub fn new(watch_paths: Vec<String>) -> Self {
        Self { watch_paths }
    }

    /// Start the watcher. Emits `(path, sha1_hex)` tuples on the channel.
    pub fn start(&self, tx: mpsc::Sender<(String, String)>) {
        let watch_paths = self.watch_paths.clone();
        let handle = tokio::runtime::Handle::current();

        std::thread::spawn(move || {
            let (notify_tx, notify_rx) = std::sync::mpsc::channel();
            let mut watcher = match notify::recommended_watcher(notify_tx) {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Failed to create file watcher: {:?}", e);
                    return;
                }
            };

            let mut any_watching = false;
            for path in &watch_paths {
                let p = Path::new(path);
                if p.exists() {
                    match watcher.watch(p, RecursiveMode::Recursive) {
                        Ok(_) => {
                            log::info!("Watching: {}", path);
                            any_watching = true;
                        }
                        Err(e) => log::warn!("Failed to watch '{}': {:?}", path, e),
                    }
                } else {
                    log::warn!("Watch path does not exist, skipping: {}", path);
                }
            }

            if !any_watching {
                log::warn!("No valid watch paths. File monitoring is inactive.");
                return;
            }

            // Debounce map: path -> last seen instant
            let mut debounce_map: HashMap<String, Instant> = HashMap::new();

            for res in notify_rx {
                match res {
                    Ok(event) => {
                        let is_relevant = matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_)
                        );
                        // Also handle renames (like on_moved in Python)
                        let is_move = matches!(event.kind, EventKind::Modify(_));
                        let _ = is_move;

                        if is_relevant {
                            for path in event.paths {
                                // Skip directories
                                if path.is_dir() { continue; }

                                let ext = path.extension()
                                    .map(|e| e.to_string_lossy().to_lowercase());
                                let ext_str = ext.as_deref().unwrap_or("");

                                if !MEDIA_EXTENSIONS.contains(&ext_str) {
                                    log::debug!("Ignored (extension): {:?}", path);
                                    continue;
                                }

                                let path_str = path.to_string_lossy().into_owned();
                                let now = Instant::now();
                                let debounce_ok = debounce_map
                                    .get(&path_str)
                                    .map(|last| now.duration_since(*last) > Duration::from_secs(2))
                                    .unwrap_or(true);

                                if !debounce_ok {
                                    log::debug!("Debounced: {}", path_str);
                                    continue;
                                }

                                // Cleanup debounce map periodically to prevent unbounded growth
                                if debounce_map.len() > 1000 {
                                    let cutoff = now - Duration::from_secs(60);
                                    debounce_map.retain(|_, last| *last > cutoff);
                                }

                                log::info!("New file event: {}", path_str);
                                debounce_map.insert(path_str.clone(), now);

                                let tx_clone = tx.clone();
                                handle.spawn(async move {
                                    match wait_for_file_completion(&path_str).await {
                                        true => {
                                            let p_clone = path_str.clone();
                                            match tokio::task::spawn_blocking(move || compute_sha1_chunked(&p_clone)).await.unwrap() {
                                                Ok(checksum) => {
                                                    log::info!("File ready: {} (sha1={})", path_str, checksum);
                                                    let _ = tx_clone.send((path_str, checksum)).await;
                                                }
                                                Err(e) => log::error!("Checksum error for {}: {}", path_str, e),
                                            }
                                        }
                                        false => log::warn!("File never stabilised, skipping: {}", path_str),
                                    }
                                });
                            }
                        }
                    }
                    Err(e) => log::error!("Watch error: {:?}", e),
                }
            }

            log::warn!("File watcher thread exiting.");
        });
    }
}

/// Wait for a file's size to remain unchanged for REQUIRED_STABLE_COUNTS checks.
/// Mirrors Python's `wait_for_file_completion`.
async fn wait_for_file_completion(path: &str) -> bool {
    let mut last_size: i64 = -1;
    let mut stable_count: u32 = 0;
    let mut last_change = Instant::now();

    loop {
        if last_change.elapsed().as_secs() >= IDLE_TIMEOUT_SECS {
            log::warn!("Timeout: file stayed inactive for {}s: {}", IDLE_TIMEOUT_SECS, path);
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

/// Compute SHA-1 in 64KB chunks — handles large files without loading all into RAM.
fn compute_sha1_chunked(path: &str) -> io::Result<String> {
    const BUF_SIZE: usize = 65536;
    let file = fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(BUF_SIZE, file);
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_compute_sha1_chunked() {
        let mut file = NamedTempFile::new().unwrap();
        // SHA1 of "hello world" is 2aae6c35c94fcfb415dbe95f408b9ce91ee846ed
        file.write_all(b"hello world").unwrap();
        
        let hash = compute_sha1_chunked(file.path().to_str().unwrap()).unwrap();
        assert_eq!(hash, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
    }
}
