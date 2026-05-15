//! Maintains a persistent index of files that have been previously synced, supporting efficient startup rescans.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::UNIX_EPOCH;

/// Number of shards for distributing sync index entries.
const SHARD_COUNT: usize = 16;

/// Represents a record stored on disk for a synced file, including the last associated album target.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SyncedFileRecord {
    pub size: u64,
    pub modified_ms: u64,
    pub checksum: String,
    #[serde(default)]
    pub album_name: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
}

/// Describes the intended album target for a file during sync operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncTarget {
    pub album_name: Option<String>,
    pub album_id: Option<String>,
}

/// Indicates the result of comparing a file on disk with the saved sync index.
pub enum SyncDecision {
    UpToDate,
    NeedsUpload,
    NeedsReassociate,
}

#[derive(Serialize, Deserialize, Default)]
struct SyncIndexData {
    files: HashMap<String, SyncedFileRecord>,
}

#[derive(Serialize)]
struct SyncIndexDataRef<'a> {
    files: &'a HashMap<String, SyncedFileRecord>,
}

/// A single shard of the sync index, holding a subset of entries.
struct Shard {
    entries: HashMap<String, SyncedFileRecord>,
    checksum_to_path: HashMap<String, String>,
    dirty: bool,
}

impl Shard {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            checksum_to_path: HashMap::new(),
            dirty: false,
        }
    }
}

/// Thread-safe sync index that distributes entries across 16 `RwLock` shards
/// keyed by path hash. This eliminates the single-lock bottleneck when
/// parallel producers (startup scan, watcher workers) and consumers (library
/// UI sync-state lookups) all contend on the index simultaneously.
///
/// The public API is identical to the old `SyncIndex` but methods are inherent
/// -- callers no longer wrap this in `Arc<Mutex<_>>`.
pub struct ShardedSyncIndex {
    index_file: PathBuf,
    shards: [RwLock<Shard>; SHARD_COUNT],
}

/// Determine which shard a path belongs to.
fn shard_for(path: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    (hasher.finish() as usize) & (SHARD_COUNT - 1)
}

impl ShardedSyncIndex {
    /// Loads the sync index from the persistent data directory, migrating
    /// from the old cache location on first run so users who clear the
    /// system cache don't lose sync state and trigger a full re-upload.
    pub fn new() -> Self {
        let index_file = crate::profile::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp").join(crate::profile::dir_segment()))
            .join("synced_index.json");

        migrate_from_cache_dir(&index_file);

        let all_entries = load_entries(&index_file);

        // Distribute entries across shards.
        let shards: [RwLock<Shard>; SHARD_COUNT] =
            std::array::from_fn(|_| RwLock::new(Shard::new()));
        for (path, record) in all_entries {
            let idx = shard_for(&path);
            let mut s = shards[idx].write().unwrap();
            s.checksum_to_path
                .insert(record.checksum.clone(), path.clone());
            s.entries.insert(path, record);
        }

        Self { index_file, shards }
    }

    /// Decide whether a file is already current, needs a new upload, or only needs reassociation.
    pub fn sync_decision(&self, path: &Path, target: &SyncTarget) -> io::Result<SyncDecision> {
        let metadata = fs::metadata(path)?;
        let fingerprint = fingerprint_from_metadata(&metadata);
        let key = path.to_string_lossy();
        let idx = shard_for(&key);
        let shard = self.shards[idx].read().unwrap();

        Ok(match shard.entries.get(key.as_ref()) {
            Some(record) => {
                if record.size != fingerprint.0 || record.modified_ms != fingerprint.1 {
                    SyncDecision::NeedsUpload
                } else if record.album_name != target.album_name
                    || record.album_id != target.album_id
                {
                    SyncDecision::NeedsReassociate
                } else {
                    SyncDecision::UpToDate
                }
            }
            None => SyncDecision::NeedsUpload,
        })
    }

    /// Save the latest synced fingerprint and album target for a file.
    pub fn record_synced(&self, path: &str, checksum: &str, target: &SyncTarget) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        let (size, modified_ms) = fingerprint_from_metadata(&metadata);
        let idx = shard_for(path);
        let mut shard = self.shards[idx].write().unwrap();
        shard.entries.insert(
            path.to_string(),
            SyncedFileRecord {
                size,
                modified_ms,
                checksum: checksum.to_string(),
                album_name: target.album_name.clone(),
                album_id: target.album_id.clone(),
            },
        );
        shard
            .checksum_to_path
            .insert(checksum.to_string(), path.to_string());
        shard.dirty = true;
        Ok(())
    }

    /// Drop records for files that no longer exist under any configured watch path.
    pub fn prune_missing(&self, seen_paths: &HashSet<String>) -> io::Result<()> {
        let mut any_changed = false;
        for lock in &self.shards {
            let mut shard = lock.write().unwrap();
            let before = shard.entries.len();
            shard.entries.retain(|path, _| seen_paths.contains(path));
            shard
                .checksum_to_path
                .retain(|_, path| seen_paths.contains(path));
            if shard.entries.len() != before {
                shard.dirty = true;
                any_changed = true;
            }
        }
        if any_changed {
            self.flush()?;
        }
        Ok(())
    }

    /// Reuse the previous checksum when a file only needs album reassociation.
    pub fn stored_checksum(&self, path: &str) -> Option<String> {
        let idx = shard_for(path);
        let shard = self.shards[idx].read().unwrap();
        shard
            .entries
            .get(path)
            .map(|record| record.checksum.clone())
    }

    /// Return the cached checksum only if the file's current size + mtime still
    /// match the stored fingerprint. Returns `None` when the file is missing,
    /// untracked, or has been modified since the last index update — callers
    /// must re-hash in that case.
    pub fn fresh_checksum(&self, path: &Path) -> Option<String> {
        let metadata = fs::metadata(path).ok()?;
        let fingerprint = fingerprint_from_metadata(&metadata);
        let key = path.to_string_lossy();
        let idx = shard_for(&key);
        let shard = self.shards[idx].read().unwrap();
        let record = shard.entries.get(key.as_ref())?;
        if record.size == fingerprint.0 && record.modified_ms == fingerprint.1 {
            Some(record.checksum.clone())
        } else {
            None
        }
    }

    /// Return a synced record for a specific path without touching the filesystem.
    pub fn record_for_path(&self, path: &str) -> Option<SyncedFileRecord> {
        let idx = shard_for(path);
        let shard = self.shards[idx].read().unwrap();
        shard.entries.get(path).cloned()
    }

    /// Remove one synced path from the index after a deliberate delete sync.
    pub fn remove_path(&self, path: &str) -> io::Result<bool> {
        let idx = shard_for(path);
        let mut shard = self.shards[idx].write().unwrap();
        let Some(record) = shard.entries.remove(path) else {
            return Ok(false);
        };
        let should_remove_checksum = shard
            .checksum_to_path
            .get(&record.checksum)
            .is_some_and(|record_path| record_path == path);
        if should_remove_checksum {
            shard.checksum_to_path.remove(&record.checksum);
        }
        shard.dirty = true;
        drop(shard);
        self.flush()?;
        Ok(true)
    }

    /// Return synced records rooted under a watch path.
    pub fn records_under_path(&self, root: &Path) -> Vec<(String, SyncedFileRecord)> {
        let mut records = Vec::new();
        for lock in &self.shards {
            let shard = lock.read().unwrap();
            records.extend(
                shard
                    .entries
                    .iter()
                    .filter(|(path, _)| Path::new(path.as_str()).starts_with(root))
                    .map(|(path, record)| (path.clone(), record.clone())),
            );
        }
        records
    }

    /// All synced paths that share a checksum. Used to detect when an asset
    /// is referenced from more than one watch folder before trashing it.
    pub fn paths_for_checksum(&self, checksum: &str) -> Vec<String> {
        let mut paths = Vec::new();
        for lock in &self.shards {
            let shard = lock.read().unwrap();
            for (path, record) in &shard.entries {
                if record.checksum == checksum {
                    paths.push(path.clone());
                }
            }
        }
        paths
    }

    /// Reverse-lookup a local path by checksum for library sync-state indicators.
    pub fn local_path_for_checksum(&self, checksum: &str) -> Option<String> {
        // The checksum could be in any shard, so we must search all.
        for lock in &self.shards {
            let shard = lock.read().unwrap();
            if let Some(path) = shard.checksum_to_path.get(checksum) {
                return Some(path.clone());
            }
        }
        None
    }

    /// Force a flush to disk if there are unwritten changes.
    /// Dirty markers are only cleared after a successful write, so a failed
    /// write leaves the data marked dirty for the next flush attempt.
    pub fn flush(&self) -> io::Result<()> {
        let mut merged = HashMap::new();
        let mut any_dirty = false;
        for lock in &self.shards {
            let shard = lock.read().unwrap();
            if shard.dirty {
                any_dirty = true;
            }
            merged.extend(shard.entries.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        if !any_dirty {
            return Ok(());
        }
        let content = serde_json::to_string_pretty(&SyncIndexDataRef { files: &merged })?;
        crate::util::atomic_write(&self.index_file, content.as_bytes())?;
        for lock in &self.shards {
            let mut shard = lock.write().unwrap();
            shard.dirty = false;
        }
        Ok(())
    }
}

/// One-time migration of the sync index from the legacy cache_dir location
/// to the persistent data_dir. Only runs when the new file is absent and the
/// old one exists. Best-effort: failures are logged and the app continues
/// with whatever is at the new path.
fn migrate_from_cache_dir(new_path: &Path) {
    if new_path.exists() {
        return;
    }
    let Some(old_path) = crate::profile::cache_dir().map(|d| d.join("synced_index.json")) else {
        return;
    };
    if !old_path.exists() {
        return;
    }
    if let Some(parent) = new_path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        log::warn!(
            "Could not create sync index data dir '{}': {}",
            parent.display(),
            err
        );
        return;
    }
    match fs::rename(&old_path, new_path) {
        Ok(()) => log::info!(
            "Migrated sync index '{}' -> '{}'",
            old_path.display(),
            new_path.display()
        ),
        Err(rename_err) => match fs::copy(&old_path, new_path) {
            Ok(_) => {
                let _ = fs::remove_file(&old_path);
                log::info!(
                    "Copied sync index '{}' -> '{}' (rename failed: {})",
                    old_path.display(),
                    new_path.display(),
                    rename_err
                );
            }
            Err(copy_err) => log::warn!(
                "Sync index migration failed (rename: {}, copy: {})",
                rename_err,
                copy_err
            ),
        },
    }
}

/// Load the saved index file, falling back to an empty index if it is missing or invalid.
fn load_entries(index_file: &Path) -> HashMap<String, SyncedFileRecord> {
    match fs::read_to_string(index_file) {
        Ok(content) => match serde_json::from_str::<SyncIndexData>(&content) {
            Ok(data) => data.files,
            Err(err) => {
                log::warn!(
                    "Failed to parse sync index '{}': {}",
                    index_file.display(),
                    err
                );
                HashMap::new()
            }
        },
        Err(_) => HashMap::new(),
    }
}

/// Reduce file metadata to the fields Mimick uses to detect local changes cheaply.
fn fingerprint_from_metadata(metadata: &fs::Metadata) -> (u64, u64) {
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();

    (metadata.len(), modified_ms)
}

#[cfg(test)]
mod tests {
    use super::{Shard, ShardedSyncIndex, SyncDecision, SyncTarget, load_entries, shard_for};
    use std::collections::HashSet;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    fn make_index(dir: &std::path::Path) -> ShardedSyncIndex {
        ShardedSyncIndex {
            index_file: dir.join("synced_index.json"),
            shards: std::array::from_fn(|_| std::sync::RwLock::new(Shard::new())),
        }
    }

    #[test]
    fn test_record_synced_then_skip_unchanged_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();

        let index = make_index(dir.path());
        let target = SyncTarget {
            album_name: Some("Album".into()),
            album_id: Some("album-1".into()),
        };

        assert!(matches!(
            index.sync_decision(&file_path, &target).unwrap(),
            SyncDecision::NeedsUpload
        ));
        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &target)
            .unwrap();
        assert!(matches!(
            index.sync_decision(&file_path, &target).unwrap(),
            SyncDecision::UpToDate
        ));
    }

    #[test]
    fn test_modified_file_needs_resync() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();

        let index = make_index(dir.path());
        let target = SyncTarget {
            album_name: Some("Album".into()),
            album_id: Some("album-1".into()),
        };
        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &target)
            .unwrap();

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&file_path)
            .unwrap();
        file.write_all(b" world").unwrap();

        assert!(matches!(
            index.sync_decision(&file_path, &target).unwrap(),
            SyncDecision::NeedsUpload
        ));
    }

    #[test]
    fn test_prune_missing_removes_deleted_entries() {
        let dir = tempdir().unwrap();
        let index = make_index(dir.path());

        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();
        let target = SyncTarget {
            album_name: Some("Album".into()),
            album_id: Some("album-1".into()),
        };
        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &target)
            .unwrap();

        index.prune_missing(&HashSet::new()).unwrap();
        assert!(matches!(
            index.sync_decision(&file_path, &target).unwrap(),
            SyncDecision::NeedsUpload
        ));
    }

    #[test]
    fn test_album_change_requires_reassociate() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();

        let index = make_index(dir.path());
        let original = SyncTarget {
            album_name: Some("Album A".into()),
            album_id: Some("album-a".into()),
        };
        let updated = SyncTarget {
            album_name: Some("Album B".into()),
            album_id: Some("album-b".into()),
        };

        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &original)
            .unwrap();

        assert!(matches!(
            index.sync_decision(&file_path, &updated).unwrap(),
            SyncDecision::NeedsReassociate
        ));
    }

    #[test]
    fn test_local_path_for_checksum_returns_matching_entry() {
        let dir = tempdir().unwrap();
        let index = make_index(dir.path());
        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();
        let target = SyncTarget {
            album_name: None,
            album_id: None,
        };
        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &target)
            .unwrap();

        assert_eq!(
            index.local_path_for_checksum("hash1"),
            Some(file_path.to_string_lossy().to_string())
        );
        assert!(index.local_path_for_checksum("missing").is_none());
    }

    #[test]
    fn test_sharded_index_distributes_entries_across_shards() {
        let paths = [
            "/home/user/photos/a.jpg",
            "/home/user/photos/b.jpg",
            "/home/user/photos/c.jpg",
            "/home/user/photos/d.jpg",
            "/home/user/photos/e.jpg",
            "/home/user/photos/f.jpg",
            "/home/user/photos/g.jpg",
            "/home/user/photos/h.jpg",
        ];
        let mut shard_ids: HashSet<usize> = HashSet::new();
        for path in &paths {
            shard_ids.insert(shard_for(path));
        }
        // With 8 paths and 16 shards, we should have at least 2 distinct shards.
        assert!(
            shard_ids.len() >= 2,
            "Expected at least 2 distinct shards, got {}",
            shard_ids.len()
        );
    }

    #[test]
    fn test_sharded_index_flush_round_trips() {
        let dir = tempdir().unwrap();
        let index = make_index(dir.path());

        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();
        let target = SyncTarget {
            album_name: Some("Album".into()),
            album_id: Some("album-1".into()),
        };
        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &target)
            .unwrap();
        index.flush().unwrap();

        // Load a fresh index from the same file and verify the entry survived.
        let index_file = dir.path().join("synced_index.json");
        let all = load_entries(&index_file);
        let shards = std::array::from_fn(|_| std::sync::RwLock::new(Shard::new()));
        for (path, record) in all {
            let idx = shard_for(&path);
            let mut s = shards[idx].write().unwrap();
            s.checksum_to_path
                .insert(record.checksum.clone(), path.clone());
            s.entries.insert(path, record);
        }
        let index2 = ShardedSyncIndex { index_file, shards };
        assert!(matches!(
            index2.sync_decision(&file_path, &target).unwrap(),
            SyncDecision::UpToDate
        ));
    }
}
