//! Persistent index of previously synced files used by startup rescans.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// On-disk record for a synced file and the album target it was last associated with.
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

/// The current target album a file should belong to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncTarget {
    pub album_name: Option<String>,
    pub album_id: Option<String>,
}

/// Result of comparing a file on disk against the saved sync index.
pub enum SyncDecision {
    UpToDate,
    NeedsUpload,
    NeedsReassociate,
}

#[derive(Serialize, Deserialize, Default)]
struct SyncIndexData {
    files: HashMap<String, SyncedFileRecord>,
}

pub struct SyncIndex {
    index_file: PathBuf,
    entries: HashMap<String, SyncedFileRecord>,
}

impl SyncIndex {
    /// Load the sync index from the default Mimick cache path.
    pub fn new() -> Self {
        let index_file = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("mimick")
            .join("synced_index.json");

        let entries = load_entries(&index_file);

        Self { index_file, entries }
    }

    /// Decide whether a file is already current, needs a new upload, or only needs reassociation.
    pub fn sync_decision(&self, path: &Path, target: &SyncTarget) -> io::Result<SyncDecision> {
        let metadata = fs::metadata(path)?;
        let fingerprint = fingerprint_from_metadata(&metadata);
        let key = path.to_string_lossy();

        Ok(match self.entries.get(key.as_ref()) {
            Some(record) => {
                if record.size != fingerprint.0 || record.modified_ms != fingerprint.1 {
                    SyncDecision::NeedsUpload
                } else if record.album_name != target.album_name || record.album_id != target.album_id {
                    SyncDecision::NeedsReassociate
                } else {
                    SyncDecision::UpToDate
                }
            }
            None => SyncDecision::NeedsUpload,
        })
    }

    /// Save the latest synced fingerprint and album target for a file.
    pub fn record_synced(
        &mut self,
        path: &str,
        checksum: &str,
        target: &SyncTarget,
    ) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        let (size, modified_ms) = fingerprint_from_metadata(&metadata);
        self.entries.insert(
            path.to_string(),
            SyncedFileRecord {
                size,
                modified_ms,
                checksum: checksum.to_string(),
                album_name: target.album_name.clone(),
                album_id: target.album_id.clone(),
            },
        );
        self.save()
    }

    /// Drop records for files that no longer exist under any configured watch path.
    pub fn prune_missing(&mut self, seen_paths: &HashSet<String>) -> io::Result<()> {
        let before = self.entries.len();
        self.entries.retain(|path, _| seen_paths.contains(path));

        if self.entries.len() != before {
            self.save()?;
        }

        Ok(())
    }

    /// Reuse the previous checksum when a file only needs album reassociation.
    pub fn stored_checksum(&self, path: &str) -> Option<String> {
        self.entries.get(path).map(|record| record.checksum.clone())
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.index_file.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(&SyncIndexData {
            files: self.entries.clone(),
        })?;

        let tmp_file = self.index_file.with_extension(format!(
            "tmp.{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        fs::write(&tmp_file, content)?;
        fs::rename(&tmp_file, &self.index_file)?;
        Ok(())
    }
}

/// Load the saved index file, falling back to an empty index if it is missing or invalid.
fn load_entries(index_file: &Path) -> HashMap<String, SyncedFileRecord> {
    match fs::read_to_string(index_file) {
        Ok(content) => match serde_json::from_str::<SyncIndexData>(&content) {
            Ok(data) => data.files,
            Err(err) => {
                log::warn!("Failed to parse sync index '{}': {}", index_file.display(), err);
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
    use super::{SyncDecision, SyncIndex, SyncTarget};
    use std::collections::HashSet;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_record_synced_then_skip_unchanged_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, b"hello").unwrap();

        let mut index = SyncIndex {
            index_file: dir.path().join("synced_index.json"),
            entries: Default::default(),
        };
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

        let mut index = SyncIndex {
            index_file: dir.path().join("synced_index.json"),
            entries: Default::default(),
        };
        let target = SyncTarget {
            album_name: Some("Album".into()),
            album_id: Some("album-1".into()),
        };
        index
            .record_synced(file_path.to_str().unwrap(), "hash1", &target)
            .unwrap();

        let mut file = fs::OpenOptions::new().append(true).open(&file_path).unwrap();
        file.write_all(b" world").unwrap();

        assert!(matches!(
            index.sync_decision(&file_path, &target).unwrap(),
            SyncDecision::NeedsUpload
        ));
    }

    #[test]
    fn test_prune_missing_removes_deleted_entries() {
        let dir = tempdir().unwrap();
        let mut index = SyncIndex {
            index_file: dir.path().join("synced_index.json"),
            entries: Default::default(),
        };

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

        let mut index = SyncIndex {
            index_file: dir.path().join("synced_index.json"),
            entries: Default::default(),
        };
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
}
