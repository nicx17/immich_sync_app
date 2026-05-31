//! Two-tier (memory + disk) thumbnail cache for the library grid.
//!
//! Remote thumbnails are fetched via the API client and decoded into
//! GDK textures. An LRU memory cache with a configurable byte budget
//! keeps hot textures in RAM, while a persistent disk cache avoids
//! redundant network requests across sessions. In-flight deduplication
//! ensures concurrent requests for the same asset coalesce into one load.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use gdk4::Texture;
use gdk4::prelude::TextureExt;
use glib::Bytes;
use lru::LruCache;
use tokio::sync::{Semaphore, watch};

use crate::api_client::{ImmichApiClient, ThumbnailSize};

const FALLBACK_CPUS: usize = 4;
const SMALL_LOAD_MAX: usize = 16;
const LARGE_LOAD_MAX: usize = 6;

type InflightSlot = Option<Result<Texture, String>>;
type InflightRx = watch::Receiver<InflightSlot>;
type InflightMap = Arc<Mutex<HashMap<String, InflightRx>>>;

/// Guard that cleans up in-flight channel entries on drop.
struct InflightGuard {
    inflight: InflightMap,
    key: String,
    tx: watch::Sender<InflightSlot>,
}

impl InflightGuard {
    /// Publish the load result to all listening subscribers.
    fn publish(&self, result: Result<Texture, String>) {
        let _ = self.tx.send(Some(result));
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let mut map = self.inflight.lock();
        map.remove(&self.key);
    }
}

/// Helper to await an in-flight thumbnail load result from a subscriber channel.
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

/// LRU Cache that limits memory usage based on estimated byte size.
struct SizedLruCache {
    inner: LruCache<String, Texture>,
    current_bytes: usize,
    max_bytes: usize,
    evictions_since_log: usize,
}

impl SizedLruCache {
    /// Construct a new LRU cache with a specific byte limit.
    fn new(max_bytes: usize) -> Self {
        let approx_per_entry = 256 * 1024;
        let count_cap = (max_bytes / approx_per_entry).max(8);
        Self {
            inner: LruCache::new(NonZeroUsize::new(count_cap).unwrap()),
            current_bytes: 0,
            max_bytes,
            evictions_since_log: 0,
        }
    }

    fn stats(&mut self) -> (usize, usize, usize, usize) {
        let evictions = self.evictions_since_log;
        self.evictions_since_log = 0;
        (
            self.inner.len(),
            self.current_bytes,
            self.max_bytes,
            evictions,
        )
    }

    /// Retrieve a texture from the cache if present, updating LRU recency.
    fn get(&mut self, key: &str) -> Option<Texture> {
        self.inner.get(key).cloned()
    }

    fn peek(&self, key: &str) -> Option<Texture> {
        self.inner.peek(key).cloned()
    }

    /// Insert a texture into the cache, evicting entries to respect the byte budget.
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
                self.evictions_since_log += 1;
            } else {
                break;
            }
        }
    }

    /// Clear all cached items and reset the current byte counter.
    fn clear(&mut self) {
        self.inner.clear();
        self.current_bytes = 0;
    }
}

/// A combined memory (LRU) and disk-based caching manager for remote and local assets.
pub struct ThumbnailCache {
    api_client: std::sync::Arc<ImmichApiClient>,
    memory: Mutex<SizedLruCache>,
    cache_dir: PathBuf,
    small_semaphore: Arc<Semaphore>,
    large_semaphore: Arc<Semaphore>,
    inflight: InflightMap,
}

impl ThumbnailCache {
    /// Floor for the auto-sized RAM budget, so very low-memory systems still
    /// hold a working set of decoded thumbnails.
    const AUTO_MIN_BYTES: usize = 500 * 1024 * 1024;
    const AUTO_MAX_BYTES: usize = 3 * 1024 * 1024 * 1024;
    const AUTO_FRACTION_PERCENT: usize = 20;

    /// Construct a new thumbnail cache manager. Disk pruning is handled
    /// centrally by `cache_manager` at startup, not here.
    pub fn new(api_client: std::sync::Arc<ImmichApiClient>) -> Self {
        let cache_dir = crate::profile::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp").join(crate::profile::dir_segment()))
            .join("thumbnails");

        let max_bytes = auto_memory_budget();
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(FALLBACK_CPUS);
        let small = SMALL_LOAD_MAX.min(cpus.saturating_mul(2)).max(2);
        let large = LARGE_LOAD_MAX.min(cpus).max(2);
        log::info!(
            "ThumbnailCache memory budget: {} MB (auto), concurrency small={} large={}",
            max_bytes / (1024 * 1024),
            small,
            large,
        );

        Self {
            api_client,
            memory: Mutex::new(SizedLruCache::new(max_bytes)),
            cache_dir,
            small_semaphore: Arc::new(Semaphore::new(small)),
            large_semaphore: Arc::new(Semaphore::new(large)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
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
            small_semaphore: Arc::new(Semaphore::new(SMALL_LOAD_MAX)),
            large_semaphore: Arc::new(Semaphore::new(LARGE_LOAD_MAX)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Retrieve a thumbnail texture from memory if present, updating LRU recency.
    pub fn get_cached(&self, asset_id: &str, size: ThumbnailSize) -> Option<Texture> {
        let key = cache_key(asset_id, size);
        self.memory.lock().get(&key)
    }

    /// Same as `get_cached` but does not touch LRU order. Use for read-only
    /// per-frame paint lookups.
    pub fn peek_cached(&self, asset_id: &str, size: ThumbnailSize) -> Option<Texture> {
        let key = cache_key(asset_id, size);
        self.memory.lock().peek(&key)
    }

    /// (entries, current_bytes, max_bytes, evictions_since_last_call).
    pub fn cache_stats(&self) -> (usize, usize, usize, usize) {
        self.memory.lock().stats()
    }

    fn semaphore_for(&self, size: ThumbnailSize) -> Arc<Semaphore> {
        match size {
            ThumbnailSize::Thumbnail => self.small_semaphore.clone(),
            ThumbnailSize::Preview | ThumbnailSize::Fullsize => self.large_semaphore.clone(),
        }
    }

    /// Asynchronously fetch a remote thumbnail with default non-cancellable execution.
    pub async fn load_thumbnail(
        &self,
        asset_id: &str,
        size: ThumbnailSize,
    ) -> Result<Texture, String> {
        self.load_thumbnail_cancellable(asset_id, size, || false)
            .await
    }

    /// Asynchronously fetch a remote thumbnail with cancellable hook support.
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

    /// Internal helper that performs the actual network request and disk backup logic.
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
            .semaphore_for(size)
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
            let texture = decode_to_scaled_texture(bytes, target_dim_for_bucket(size))
                .await
                .map_err(|err| err.to_string())?;
            self.memory.lock().insert(key.to_string(), texture.clone());
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
        let texture = decode_to_scaled_texture(bytes, target_dim_for_bucket(size))
            .await
            .map_err(|err| err.to_string())?;
        self.memory.lock().insert(key.to_string(), texture.clone());
        Ok(texture)
    }

    /// Asynchronously generate a local file thumbnail with cancellable hook support.
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
        if let Some(texture) = self.memory.lock().get(&key) {
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

    /// Internal helper that scales, saves, and loads a local folder asset thumbnail.
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
            .semaphore_for(ThumbnailSize::Thumbnail)
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;

        if is_cancelled() {
            return Err("cancelled".to_string());
        }
        if let Some(texture) = self.memory.lock().get(key) {
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
            self.memory.lock().insert(key.to_string(), texture.clone());
            return Ok(texture);
        }

        let decode_started = std::time::Instant::now();
        let path = path.to_path_buf();
        let log_path = path.clone();
        let cache_dir = self.cache_dir.clone();
        let texture = tokio::task::spawn_blocking(move || -> Result<Texture, String> {
            let is_raw = crate::media_kinds::is_raw_path(&path);
            // For RAW files, gdk_pixbuf never recognises the format, so skip
            // straight to the custom decoder to avoid a wasted attempt + log noise.
            let pixbuf = if is_raw {
                // Try custom RAW decoder first (skips the pixbuf attempt that
                // always fails for most camera RAW formats).
                custom_decode_to_thumbnail(&path).or_else(|_| {
                    // Some simple TIFF-based RAW files (DNG) can still be
                    // decoded by glycin/pixbuf, so fall back to it.
                    gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true)
                        .map(|raw| raw.apply_embedded_orientation().unwrap_or(raw))
                        .map_err(|e| e.to_string())
                })?
            } else {
                match gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true) {
                    Ok(raw) => raw.apply_embedded_orientation().unwrap_or(raw),
                    Err(err) => {
                        log::debug!(
                            "gdk_pixbuf direct load failed for {}: {}; attempting custom decoder",
                            path.display(),
                            err
                        );
                        custom_decode_to_thumbnail(&path)?
                    }
                }
            };
            std::fs::create_dir_all(&cache_dir).map_err(|err| err.to_string())?;
            let encoded = pixbuf_png_bytes(&pixbuf)?;
            std::fs::write(&cache_file, encoded).map_err(|err| err.to_string())?;
            let format = if pixbuf.has_alpha() {
                gdk4::MemoryFormat::R8g8b8a8
            } else {
                gdk4::MemoryFormat::R8g8b8
            };
            let bytes = pixbuf.read_pixel_bytes();
            let mem_tex = gdk4::MemoryTexture::new(
                pixbuf.width(),
                pixbuf.height(),
                format,
                &bytes,
                pixbuf.rowstride() as usize,
            );
            use gtk::prelude::Cast;
            Ok(mem_tex.upcast::<Texture>())
        })
        .await
        .map_err(|err| err.to_string())??;
        log::debug!(
            "Local thumbnail decoded fresh for {} in {}ms ({}x{})",
            log_path.display(),
            decode_started.elapsed().as_millis(),
            texture.width(),
            texture.height(),
        );
        self.memory.lock().insert(key.to_string(), texture.clone());
        Ok(texture)
    }

    /// Purge all memory caches and completely delete the disk cache directory.
    pub fn clear(&self) -> Result<(), String> {
        self.memory.lock().clear();
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir).map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    /// Drop every cached texture from RAM without touching the disk cache.
    /// Invoked when the library window closes so the texture memory is
    /// released until the user reopens it.
    pub fn clear_memory(&self) {
        self.memory.lock().clear();
    }

    /// Return the physical disk cache path for a remote asset thumbnail.
    fn cache_file(&self, asset_id: &str, size: ThumbnailSize) -> PathBuf {
        self.cache_dir.join(cache_key(asset_id, size))
    }

    /// Return the physical disk cache path for a local folder asset thumbnail.
    fn cache_file_local(&self, asset_id: &str) -> PathBuf {
        self.cache_dir.join(local_cache_key(asset_id))
    }

    /// Register or join an ongoing in-flight loading request for a key.
    fn enter_inflight(&self, key: &str) -> Result<InflightGuard, InflightRx> {
        let mut map = self.inflight.lock();
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

/// Construct a cache key string for remote assets.
fn cache_key(asset_id: &str, size: ThumbnailSize) -> String {
    match size {
        ThumbnailSize::Thumbnail => format!("thumbnail:{}", asset_id),
        ThumbnailSize::Preview => format!("preview:{}", asset_id),
        ThumbnailSize::Fullsize => format!("fullsize:{}", asset_id),
    }
}

/// Construct a cache key string for local assets using hashed paths.
fn local_cache_key(asset_id: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    asset_id.hash(&mut hasher);
    format!("local-thumbnail-v2:{:x}", hasher.finish())
}

/// Estimate memory byte size occupied by a texture.
fn estimate_texture_bytes(texture: &Texture) -> usize {
    texture.width().max(1) as usize * texture.height().max(1) as usize * 4
}

/// Pick a thumbnail-cache RAM budget from `MemTotal` in `/proc/meminfo`,
/// clamped to `[AUTO_MIN_BYTES, AUTO_MAX_BYTES]`. Falls back to the floor
/// if the file can't be read.
fn auto_memory_budget() -> usize {
    let total = read_meminfo_total_bytes().unwrap_or(0);
    if total == 0 {
        return ThumbnailCache::AUTO_MIN_BYTES;
    }
    let fraction = total / 100 * ThumbnailCache::AUTO_FRACTION_PERCENT;
    fraction.clamp(
        ThumbnailCache::AUTO_MIN_BYTES,
        ThumbnailCache::AUTO_MAX_BYTES,
    )
}

fn read_meminfo_total_bytes() -> Option<usize> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: usize = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

/// Decode a file through the custom pipeline (RAW / non-pixbuf formats) and
/// scale the result down to a 256x256 thumbnail pixbuf.
///
/// RAW paths use the thumbnail-specific decoder (embedded JPEG first, full
/// demosaic only as last resort) so the global "Full RAW Decoding" toggle
/// — which is meant for lightbox quality — never penalises grid loading.
fn custom_decode_to_thumbnail(path: &std::path::Path) -> Result<gtk::gdk_pixbuf::Pixbuf, String> {
    let full_texture = if crate::media_kinds::is_raw_path(path) {
        super::decode_raw_thumbnail_texture(path)
    } else {
        super::load_texture_blocking(path)
    }
    .ok_or_else(|| format!("No decoder succeeded for {}", path.display()))?;
    let mut downloader = gdk4::TextureDownloader::new(&full_texture);
    downloader.set_format(gdk4::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let full_pixbuf = gtk::gdk_pixbuf::Pixbuf::from_mut_slice(
        bytes.to_vec(),
        gtk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        full_texture.width(),
        full_texture.height(),
        stride as i32,
    );
    let w = full_pixbuf.width();
    let h = full_pixbuf.height();
    let scale = (256.0 / w as f64).min(256.0 / h as f64).min(1.0);
    let tw = ((w as f64 * scale).round() as i32).max(1);
    let th = ((h as f64 * scale).round() as i32).max(1);
    full_pixbuf
        .scale_simple(tw, th, gtk::gdk_pixbuf::InterpType::Bilinear)
        .ok_or_else(|| "Failed to scale pixbuf".to_string())
}

fn pixbuf_png_bytes(pixbuf: &gtk::gdk_pixbuf::Pixbuf) -> Result<Vec<u8>, String> {
    let width = pixbuf.width().max(1) as usize;
    let height = pixbuf.height().max(1) as usize;
    let channels = if pixbuf.has_alpha() { 4 } else { 3 };
    let rowstride = pixbuf.rowstride() as usize;
    let bytes = pixbuf.read_pixel_bytes();

    let packed = pack_pixel_rows(bytes.as_ref(), width, height, channels, rowstride)?;
    encode_png_bytes(&packed, width as u32, height as u32, pixbuf.has_alpha())
}

fn pack_pixel_rows(
    src: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    rowstride: usize,
) -> Result<Vec<u8>, String> {
    let mut packed = Vec::with_capacity(width * height * channels);
    for row in 0..height {
        let start = row
            .checked_mul(rowstride)
            .ok_or_else(|| "pixbuf row offset overflow".to_string())?;
        let end = start
            .checked_add(width * channels)
            .ok_or_else(|| "pixbuf row end overflow".to_string())?;
        let row_bytes = src
            .get(start..end)
            .ok_or_else(|| "pixbuf row outside buffer".to_string())?;
        packed.extend_from_slice(row_bytes);
    }
    Ok(packed)
}

fn encode_png_bytes(
    packed: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let color = if has_alpha {
        image::ColorType::Rgba8
    } else {
        image::ColorType::Rgb8
    };
    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(packed, width, height, color.into())
        .map_err(|err| err.to_string())?;
    Ok(encoded)
}

fn target_dim_for_bucket(size: ThumbnailSize) -> i32 {
    match size {
        ThumbnailSize::Thumbnail => 256,
        ThumbnailSize::Preview => 1440,
        ThumbnailSize::Fullsize => 2560,
    }
}

/// Asynchronously decode image bytes into a scaled texture.
async fn decode_to_scaled_texture(bytes: Vec<u8>, max_dim: i32) -> Result<Texture, String> {
    tokio::task::spawn_blocking(move || -> Result<Texture, String> {
        let stream = gtk::gio::MemoryInputStream::from_bytes(&Bytes::from_owned(bytes));
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
            &stream,
            max_dim,
            max_dim,
            true,
            gtk::gio::Cancellable::NONE,
        )
        .map_err(|err| err.to_string())?;
        let format = if pixbuf.has_alpha() {
            gdk4::MemoryFormat::R8g8b8a8
        } else {
            gdk4::MemoryFormat::R8g8b8
        };
        let bytes = pixbuf.read_pixel_bytes();
        let mem_tex = gdk4::MemoryTexture::new(
            pixbuf.width(),
            pixbuf.height(),
            format,
            &bytes,
            pixbuf.rowstride() as usize,
        );
        use gtk::prelude::Cast;
        Ok(mem_tex.upcast::<Texture>())
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
            .insert("thumbnail:1".into(), texture_from_png());
        cache
            .memory
            .lock()
            .insert("thumbnail:2".into(), texture_from_png());

        assert!(cache.memory.lock().inner.len() <= 1);
    }

    #[test]
    fn test_clear_removes_memory_and_disk() {
        let cache = cache(1024);
        std::fs::create_dir_all(&cache.cache_dir).unwrap();
        std::fs::write(cache.cache_file("3", ThumbnailSize::Thumbnail), PNG_BYTES).unwrap();
        cache
            .memory
            .lock()
            .insert("thumbnail:3".into(), texture_from_png());

        cache.clear().unwrap();

        assert!(cache.memory.lock().inner.is_empty());
        assert!(!cache.cache_dir.exists());
    }

    #[tokio::test]
    async fn test_load_local_raw_thumbnail() {
        let cache = cache(1024 * 1024 * 10);
        let fixture_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.dng");

        let result = cache
            .load_local_thumbnail_cancellable("local_dng_test", &fixture_path, || false)
            .await;

        assert!(
            result.is_ok(),
            "Failed to load RAW thumbnail: {:?}",
            result.err()
        );
        let texture = result.unwrap();
        assert!(texture.width() > 0);
        assert!(texture.height() > 0);

        let cache_file = cache.cache_file_local("local_dng_test");
        assert!(cache_file.exists(), "Cache file was not written to disk");
    }
}
