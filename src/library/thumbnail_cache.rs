//! Thumbnail cache with in-memory LRU (byte-budget) and on-disk persistence.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex;

use gdk4::Texture;
use gdk4::prelude::TextureExt;
use glib::Bytes;
use lru::LruCache;

use crate::api_client::{ImmichApiClient, ThumbnailSize};

#[derive(Clone)]
struct CachedBytes {
    bytes: Vec<u8>,
    estimated_bytes: usize,
}

struct SizedLruCache {
    inner: LruCache<String, CachedBytes>,
    current_bytes: usize,
    max_bytes: usize,
}

impl SizedLruCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            inner: LruCache::new(NonZeroUsize::new(1024).unwrap()),
            current_bytes: 0,
            max_bytes,
        }
    }

    fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        self.inner.get(key).map(|entry| entry.bytes.clone())
    }

    fn insert(&mut self, key: String, entry: CachedBytes) {
        if let Some(previous) = self.inner.put(key, entry.clone()) {
            self.current_bytes = self.current_bytes.saturating_sub(previous.estimated_bytes);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry.estimated_bytes);
        while self.current_bytes > self.max_bytes {
            if let Some((_key, removed)) = self.inner.pop_lru() {
                self.current_bytes = self.current_bytes.saturating_sub(removed.estimated_bytes);
            } else {
                break;
            }
        }
    }

    fn clear(&mut self) {
        self.inner.clear();
        self.current_bytes = 0;
    }
}

pub struct ThumbnailCache {
    api_client: std::sync::Arc<ImmichApiClient>,
    memory: Mutex<SizedLruCache>,
    cache_dir: PathBuf,
}

impl ThumbnailCache {
    const DEFAULT_MAX_BYTES: usize = 80 * 1024 * 1024;

    pub fn new(api_client: std::sync::Arc<ImmichApiClient>) -> Self {
        Self::with_capacity_mb(api_client, 0)
    }

    /// Build a cache with a configured byte budget. `mb == 0` falls back to
    /// `DEFAULT_MAX_BYTES` so the existing `new()` callers keep working.
    pub fn with_capacity_mb(api_client: std::sync::Arc<ImmichApiClient>, mb: u32) -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("mimick")
            .join("thumbnails");

        let max_bytes = if mb == 0 {
            Self::DEFAULT_MAX_BYTES
        } else {
            (mb as usize).saturating_mul(1024 * 1024)
        };

        let cache = Self {
            api_client,
            memory: Mutex::new(SizedLruCache::new(max_bytes)),
            cache_dir,
        };
        let _ = cache.prune_disk_cache(500 * 1024 * 1024);
        cache
    }

    #[cfg(test)]
    fn new_for_test(
        api_client: std::sync::Arc<ImmichApiClient>,
        cache_dir: PathBuf,
        max_bytes: usize,
    ) -> Self {
        Self {
            api_client,
            memory: Mutex::new(SizedLruCache::new(max_bytes)),
            cache_dir,
        }
    }

    pub fn get_cached(&self, asset_id: &str, size: ThumbnailSize) -> Option<Texture> {
        let key = cache_key(asset_id, size);
        if let Some(bytes) = self.memory.lock().unwrap().get(&key) {
            return decode_texture(&bytes).ok();
        }

        let path = self.cache_file(asset_id, size);
        let bytes = std::fs::read(path).ok()?;
        let texture = decode_texture(&bytes).ok()?;
        let estimated_bytes = estimate_texture_bytes(&texture);
        self.memory.lock().unwrap().insert(
            key,
            CachedBytes {
                bytes,
                estimated_bytes,
            },
        );
        Some(texture)
    }

    pub async fn load_thumbnail(
        &self,
        asset_id: &str,
        size: ThumbnailSize,
    ) -> Result<Texture, String> {
        if let Some(texture) = self.get_cached(asset_id, size) {
            return Ok(texture);
        }

        let bytes = self.api_client.fetch_thumbnail(asset_id, size).await?;
        let texture = decode_texture(&bytes)?;
        let estimated_bytes = estimate_texture_bytes(&texture);

        if let Some(parent) = self.cache_dir.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::create_dir_all(&self.cache_dir).map_err(|err| err.to_string())?;
        std::fs::write(self.cache_file(asset_id, size), &bytes).map_err(|err| err.to_string())?;
        self.memory.lock().unwrap().insert(
            cache_key(asset_id, size),
            CachedBytes {
                bytes,
                estimated_bytes,
            },
        );

        Ok(texture)
    }

    pub fn clear(&self) -> Result<(), String> {
        self.memory.lock().unwrap().clear();
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir).map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    fn prune_disk_cache(&self, max_bytes: u64) -> Result<(), String> {
        if !self.cache_dir.exists() {
            return Ok(());
        }

        let mut entries = Vec::new();
        let mut total_size = 0u64;

        if let Ok(dir) = std::fs::read_dir(&self.cache_dir) {
            for entry in dir.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    let size = metadata.len();
                    let modified = metadata
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    total_size += size;
                    entries.push((entry.path(), size, modified));
                }
            }
        }

        if total_size <= max_bytes {
            return Ok(());
        }

        // Sort by oldest first
        entries.sort_by_key(|a| a.2);

        for (path, size, _) in entries {
            if total_size <= max_bytes {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total_size = total_size.saturating_sub(size);
            }
        }

        Ok(())
    }

    fn cache_file(&self, asset_id: &str, size: ThumbnailSize) -> PathBuf {
        self.cache_dir.join(cache_key(asset_id, size))
    }
}

fn cache_key(asset_id: &str, size: ThumbnailSize) -> String {
    match size {
        ThumbnailSize::Thumbnail => format!("thumbnail:{}", asset_id),
        ThumbnailSize::Preview => format!("preview:{}", asset_id),
    }
}

fn decode_texture(bytes: &[u8]) -> Result<Texture, String> {
    Texture::from_bytes(&Bytes::from(bytes)).map_err(|err| err.to_string())
}

fn estimate_texture_bytes(texture: &Texture) -> usize {
    texture.width().max(1) as usize * texture.height().max(1) as usize * 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::ImmichApiClient;
    use tempfile::tempdir;

    // 1x1 transparent PNG
    const PNG_BYTES: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    fn cache(max_bytes: usize) -> ThumbnailCache {
        let dir = tempdir().unwrap();
        let cache_dir = dir.keep().join("thumbs");
        ThumbnailCache::new_for_test(
            std::sync::Arc::new(ImmichApiClient::new(
                String::new(),
                String::new(),
                String::new(),
            )),
            cache_dir,
            max_bytes,
        )
    }

    #[test]
    fn test_memory_hit_after_insert() {
        let cache = cache(1024);
        let texture = decode_texture(PNG_BYTES).unwrap();
        cache.memory.lock().unwrap().insert(
            "thumbnail:1".into(),
            CachedBytes {
                bytes: PNG_BYTES.to_vec(),
                estimated_bytes: estimate_texture_bytes(&texture),
            },
        );

        assert!(cache.get_cached("1", ThumbnailSize::Thumbnail).is_some());
    }

    #[test]
    fn test_disk_hit_reads_and_populates_memory() {
        let cache = cache(1024);
        std::fs::create_dir_all(&cache.cache_dir).unwrap();
        std::fs::write(cache.cache_file("2", ThumbnailSize::Thumbnail), PNG_BYTES).unwrap();

        assert!(cache.get_cached("2", ThumbnailSize::Thumbnail).is_some());
        assert!(cache.memory.lock().unwrap().get("thumbnail:2").is_some());
    }

    #[test]
    fn test_eviction_after_byte_budget_overflow() {
        let cache = cache(4);
        let texture = decode_texture(PNG_BYTES).unwrap();
        let entry = CachedBytes {
            bytes: PNG_BYTES.to_vec(),
            estimated_bytes: estimate_texture_bytes(&texture),
        };
        cache
            .memory
            .lock()
            .unwrap()
            .insert("thumbnail:1".into(), entry.clone());
        cache
            .memory
            .lock()
            .unwrap()
            .insert("thumbnail:2".into(), entry);

        assert!(cache.memory.lock().unwrap().inner.len() <= 1);
    }

    #[test]
    fn test_clear_removes_memory_and_disk() {
        let cache = cache(1024);
        std::fs::create_dir_all(&cache.cache_dir).unwrap();
        std::fs::write(cache.cache_file("3", ThumbnailSize::Thumbnail), PNG_BYTES).unwrap();
        cache.memory.lock().unwrap().insert(
            "thumbnail:3".into(),
            CachedBytes {
                bytes: PNG_BYTES.to_vec(),
                estimated_bytes: 4,
            },
        );

        cache.clear().unwrap();

        assert!(cache.memory.lock().unwrap().inner.is_empty());
        assert!(!cache.cache_dir.exists());
    }
}
