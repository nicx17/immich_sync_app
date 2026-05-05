use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gdk4::Texture;
use gdk4::prelude::TextureExt;
use glib::Bytes;
use lru::LruCache;
use tokio::sync::{Semaphore, watch};

use crate::api_client::{ImmichApiClient, ThumbnailSize};
const MAX_CONCURRENT_LOADS: usize = 8;

type InflightSlot = Option<Result<Texture, String>>;
type InflightRx = watch::Receiver<InflightSlot>;
type InflightMap = Arc<Mutex<HashMap<String, InflightRx>>>;

struct InflightGuard {
    inflight: InflightMap,
    key: String,
    tx: watch::Sender<InflightSlot>,
}

impl InflightGuard {
    fn publish(&self, result: Result<Texture, String>) {
        let _ = self.tx.send(Some(result));
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.inflight.lock() {
            map.remove(&self.key);
        }
    }
}

async fn await_inflight(mut rx: InflightRx) -> Result<Texture, String> {
    if let Some(result) = rx.borrow_and_update().clone() {
        return result;
    }
    match rx.changed().await {
        Ok(()) => rx
            .borrow()
            .clone()
            .unwrap_or_else(|| Err("Thumbnail load cancelled".to_string())),
        Err(_) => Err("Thumbnail load cancelled".to_string()),
    }
}

struct SizedLruCache {
    inner: LruCache<String, Texture>,
    current_bytes: usize,
    max_bytes: usize,
}

impl SizedLruCache {
    fn new(max_bytes: usize) -> Self {
        let approx_per_entry = 256 * 1024;
        let count_cap = (max_bytes / approx_per_entry).max(8);
        Self {
            inner: LruCache::new(NonZeroUsize::new(count_cap).unwrap()),
            current_bytes: 0,
            max_bytes,
        }
    }

    fn get(&mut self, key: &str) -> Option<Texture> {
        self.inner.get(key).cloned()
    }

    fn insert(&mut self, key: String, texture: Texture) {
        let added = estimate_texture_bytes(&texture);
        if let Some(previous) = self.inner.put(key, texture) {
            self.current_bytes = self
                .current_bytes
                .saturating_sub(estimate_texture_bytes(&previous));
        }
        self.current_bytes = self.current_bytes.saturating_add(added);
        while self.current_bytes > self.max_bytes {
            if let Some((_key, removed)) = self.inner.pop_lru() {
                self.current_bytes = self
                    .current_bytes
                    .saturating_sub(estimate_texture_bytes(&removed));
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
    load_semaphore: Arc<Semaphore>,
    inflight: InflightMap,
}

impl ThumbnailCache {
    const DEFAULT_MAX_BYTES: usize = 80 * 1024 * 1024;

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
            load_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_LOADS)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
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
            load_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_LOADS)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_cached(&self, asset_id: &str, size: ThumbnailSize) -> Option<Texture> {
        let key = cache_key(asset_id, size);
        self.memory.lock().unwrap().get(&key)
    }

    pub async fn load_thumbnail(
        &self,
        asset_id: &str,
        size: ThumbnailSize,
    ) -> Result<Texture, String> {
        self.load_thumbnail_cancellable(asset_id, size, || false)
            .await
    }

    pub async fn load_thumbnail_cancellable<F>(
        &self,
        asset_id: &str,
        size: ThumbnailSize,
        is_cancelled: F,
    ) -> Result<Texture, String>
    where
        F: Fn() -> bool,
    {
        if let Some(texture) = self.get_cached(asset_id, size) {
            return Ok(texture);
        }

        let key = cache_key(asset_id, size);
        let guard = match self.enter_inflight(&key) {
            Ok(guard) => guard,
            Err(rx) => return await_inflight(rx).await,
        };
        let result = self
            .fetch_remote_thumbnail(asset_id, size, &key, &is_cancelled)
            .await;
        guard.publish(result.clone());
        result
    }

    async fn fetch_remote_thumbnail(
        &self,
        asset_id: &str,
        size: ThumbnailSize,
        key: &str,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Texture, String> {
        if is_cancelled() {
            return Err("cancelled".to_string());
        }
        let _permit = self
            .load_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;

        if is_cancelled() {
            return Err("cancelled".to_string());
        }
        if let Some(texture) = self.get_cached(asset_id, size) {
            return Ok(texture);
        }

        let cache_file = self.cache_file(asset_id, size);
        let cache_file_for_read = cache_file.clone();
        let from_disk = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
            std::fs::read(&cache_file_for_read).ok()
        })
        .await
        .map_err(|err| err.to_string())?;

        if let Some(bytes) = from_disk {
            let texture = decode_to_scaled_texture(bytes)
                .await
                .map_err(|err| err.to_string())?;
            self.memory
                .lock()
                .unwrap()
                .insert(key.to_string(), texture.clone());
            return Ok(texture);
        }

        if is_cancelled() {
            return Err("cancelled".to_string());
        }
        let bytes = self.api_client.fetch_thumbnail(asset_id, size).await?;
        let cache_dir = self.cache_dir.clone();
        let cache_file_for_write = cache_file.clone();
        let bytes_for_write = bytes.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = std::fs::create_dir_all(&cache_dir);
            let _ = std::fs::write(&cache_file_for_write, &bytes_for_write);
        })
        .await;
        let texture = decode_to_scaled_texture(bytes)
            .await
            .map_err(|err| err.to_string())?;

        self.memory
            .lock()
            .unwrap()
            .insert(key.to_string(), texture.clone());
        Ok(texture)
    }

    #[allow(dead_code)]
    pub async fn load_local_thumbnail(
        &self,
        asset_id: &str,
        path: &std::path::Path,
    ) -> Result<Texture, String> {
        self.load_local_thumbnail_cancellable(asset_id, path, || false)
            .await
    }

    pub async fn load_local_thumbnail_cancellable<F>(
        &self,
        asset_id: &str,
        path: &std::path::Path,
        is_cancelled: F,
    ) -> Result<Texture, String>
    where
        F: Fn() -> bool,
    {
        let key = cache_key(asset_id, ThumbnailSize::Thumbnail);
        if let Some(texture) = self.memory.lock().unwrap().get(&key) {
            return Ok(texture);
        }

        let guard = match self.enter_inflight(&key) {
            Ok(guard) => guard,
            Err(rx) => return await_inflight(rx).await,
        };
        let result = self
            .fetch_local_thumbnail(asset_id, path, &key, &is_cancelled)
            .await;
        guard.publish(result.clone());
        result
    }

    async fn fetch_local_thumbnail(
        &self,
        asset_id: &str,
        path: &std::path::Path,
        key: &str,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Texture, String> {
        if is_cancelled() {
            return Err("cancelled".to_string());
        }
        let _permit = self
            .load_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;

        if is_cancelled() {
            return Err("cancelled".to_string());
        }
        if let Some(texture) = self.memory.lock().unwrap().get(key) {
            return Ok(texture);
        }

        let cache_file = self.cache_file_local(asset_id);
        let cache_file_for_read = cache_file.clone();
        let from_disk = tokio::task::spawn_blocking(move || -> Option<Texture> {
            if !cache_file_for_read.exists() {
                return None;
            }
            Texture::from_filename(&cache_file_for_read).ok()
        })
        .await
        .map_err(|err| err.to_string())?;

        if let Some(texture) = from_disk {
            self.memory
                .lock()
                .unwrap()
                .insert(key.to_string(), texture.clone());
            return Ok(texture);
        }

        let path = path.to_path_buf();
        let cache_dir = self.cache_dir.clone();
        let texture = tokio::task::spawn_blocking(move || -> Result<Texture, String> {
            let raw = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true)
                .map_err(|err| err.to_string())?;
            let pixbuf = raw.apply_embedded_orientation().unwrap_or(raw);
            let _ = std::fs::create_dir_all(&cache_dir);
            let _ = pixbuf.savev(&cache_file, "png", &[]);
            #[allow(deprecated)]
            Ok(Texture::for_pixbuf(&pixbuf))
        })
        .await
        .map_err(|err| err.to_string())??;
        self.memory
            .lock()
            .unwrap()
            .insert(key.to_string(), texture.clone());
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

    fn cache_file_local(&self, asset_id: &str) -> PathBuf {
        self.cache_dir.join(local_cache_key(asset_id))
    }

    fn enter_inflight(&self, key: &str) -> Result<InflightGuard, InflightRx> {
        let mut map = self.inflight.lock().unwrap();
        if let Some(rx) = map.get(key) {
            return Err(rx.clone());
        }
        let (tx, rx) = watch::channel::<InflightSlot>(None);
        map.insert(key.to_string(), rx);
        Ok(InflightGuard {
            inflight: self.inflight.clone(),
            key: key.to_string(),
            tx,
        })
    }
}

fn cache_key(asset_id: &str, size: ThumbnailSize) -> String {
    match size {
        ThumbnailSize::Thumbnail => format!("thumbnail:{}", asset_id),
        ThumbnailSize::Preview => format!("preview:{}", asset_id),
    }
}

fn local_cache_key(asset_id: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    asset_id.hash(&mut hasher);
    format!("local-thumbnail-v2:{:x}", hasher.finish())
}

fn estimate_texture_bytes(texture: &Texture) -> usize {
    texture.width().max(1) as usize * texture.height().max(1) as usize * 4
}

async fn decode_to_scaled_texture(bytes: Vec<u8>) -> Result<Texture, String> {
    tokio::task::spawn_blocking(move || -> Result<Texture, String> {
        let stream = gtk::gio::MemoryInputStream::from_bytes(&Bytes::from_owned(bytes));
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
            &stream,
            256,
            256,
            true,
            gtk::gio::Cancellable::NONE,
        )
        .map_err(|err| err.to_string())?;
        #[allow(deprecated)]
        Ok(Texture::for_pixbuf(&pixbuf))
    })
    .await
    .map_err(|err| err.to_string())?
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

    fn texture_from_png() -> Texture {
        Texture::from_bytes(&Bytes::from(PNG_BYTES)).unwrap()
    }

    #[test]
    fn test_memory_hit_after_insert() {
        let cache = cache(1024);
        cache
            .memory
            .lock()
            .unwrap()
            .insert("thumbnail:1".into(), texture_from_png());

        assert!(cache.get_cached("1", ThumbnailSize::Thumbnail).is_some());
    }

    #[test]
    fn test_get_cached_does_not_touch_disk() {
        let cache = cache(1024);
        std::fs::create_dir_all(&cache.cache_dir).unwrap();
        std::fs::write(cache.cache_file("2", ThumbnailSize::Thumbnail), PNG_BYTES).unwrap();

        assert!(cache.get_cached("2", ThumbnailSize::Thumbnail).is_none());
    }

    #[test]
    fn test_eviction_after_byte_budget_overflow() {
        let cache = cache(3);
        cache
            .memory
            .lock()
            .unwrap()
            .insert("thumbnail:1".into(), texture_from_png());
        cache
            .memory
            .lock()
            .unwrap()
            .insert("thumbnail:2".into(), texture_from_png());

        assert!(cache.memory.lock().unwrap().inner.len() <= 1);
    }

    #[test]
    fn test_clear_removes_memory_and_disk() {
        let cache = cache(1024);
        std::fs::create_dir_all(&cache.cache_dir).unwrap();
        std::fs::write(cache.cache_file("3", ThumbnailSize::Thumbnail), PNG_BYTES).unwrap();
        cache
            .memory
            .lock()
            .unwrap()
            .insert("thumbnail:3".into(), texture_from_png());

        cache.clear().unwrap();

        assert!(cache.memory.lock().unwrap().inner.is_empty());
        assert!(!cache.cache_dir.exists());
    }
}
