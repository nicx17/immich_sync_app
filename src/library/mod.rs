//! Library view module -- browse, search, and download assets from an Immich server.
//!
//! Constructs the main GTK window with a split-pane sidebar, paginated
//! grid view, timeline scrubber, and search bar. Submodules handle the
//! explore dashboard, album grids, lightbox, sidebar, and thumbnail
//! caching independently.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{LibraryAsset, MetadataSearchFilters};
use crate::app_context::AppContext;
use crate::library::albums_view::{
    AlbumClick, AlbumsViewParts, build_albums_view, populate_albums,
};
use crate::library::explore_view::{ExploreViewParts, build_explore_view};
use crate::library::local_source::{
    LocalAsset, enumerate_local, enumerate_local_for_entry, filter_by_filename,
};
use crate::library::masonry::{GridViewParts, build_grid_view};
use crate::library::sidebar::{SidebarParts, build_sidebar};
use crate::library::state::{LibraryLoadState, LibrarySource};
use crate::state_manager::TransferDirection;

use self::actions::{connect_bulk_actions, connect_select_mode};
use self::album_link::{connect_album_link_row, refresh_album_link_row};
use self::context_menu::show_asset_context_menu;
use self::controls::{
    connect_controls, connect_grid_handlers, connect_sidebar_handlers,
    refresh_library_after_mutation, sidebar_dispatch,
};
use self::download::format_rate;
use self::filters::connect_filters_button;
use self::lightbox::open_lightbox;

pub(super) const LOCAL_ID_PREFIX: &str = "local::";

pub mod album_sync;
pub mod albums_view;
pub mod asset_model;
pub mod asset_object;
pub mod explore_view;
pub mod local_exif;
pub mod local_source;
pub mod masonry;
pub mod sidebar;
pub mod state;
pub mod style;
pub mod thumbnail_cache;

mod actions;
mod album_link;
mod context_menu;
mod controls;
mod download;
mod filters;
mod lightbox;
mod server_stats_dialog;
mod upload_picker;

const PAGE_SIZE: u32 = 50;

/// Runtime flag controlling the on-disk RAW decode cache.
/// Initialised from config at startup; toggled live from the settings UI.
static RAW_CACHE_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Update the runtime RAW cache flag (called from settings and startup).
pub fn set_raw_cache_enabled(enabled: bool) {
    RAW_CACHE_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Runtime flag: true = full demosaic, false = extract embedded JPEG preview.
static RAW_FULL_DECODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Update the runtime RAW full-decode flag (called from settings and startup).
pub fn set_raw_full_decode(enabled: bool) {
    RAW_FULL_DECODE.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Register application custom icons with the Gtk icon theme system.
fn register_app_icons() {
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        theme.add_resource_path("/dev/nicx/mimick/icons");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextureDecoder {
    Raw,
    Heif,
    JpegXl,
    Svg,
    Jpeg,
    Webp,
    Jpeg2k,
    Psd,
    Pixbuf,
    ImageFallback,
}

/// Load a local texture, preferring decoders that cover formats outside the
/// GDK-Pixbuf runtime loader set before falling back to pixbuf and image-rs.
async fn load_texture_oriented(path: &std::path::Path) -> Option<gdk4::Texture> {
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || load_texture_blocking(&path_buf))
        .await
        .ok()
        .flatten()
}

pub(super) fn load_texture_blocking(path: &std::path::Path) -> Option<gdk4::Texture> {
    let started = std::time::Instant::now();
    let decoder = texture_decoder_for_path(path);
    let mut winning_route = decoder;
    let texture = match decoder {
        TextureDecoder::Raw => decode_raw_texture(path),
        TextureDecoder::Heif => decode_heif_texture(path),
        TextureDecoder::JpegXl => decode_jpegxl_texture(path),
        TextureDecoder::Svg => decode_svg_texture(path),
        TextureDecoder::Jpeg => decode_jpeg_texture(path),
        TextureDecoder::Webp => decode_webp_texture(path),
        TextureDecoder::Jpeg2k => decode_jpeg2k_texture(path),
        TextureDecoder::Psd => decode_psd_texture(path),
        TextureDecoder::Pixbuf => decode_pixbuf_texture(path),
        TextureDecoder::ImageFallback => None,
    };

    let result = texture
        .or_else(|| {
            if decoder != TextureDecoder::Pixbuf {
                let fallback = decode_pixbuf_texture(path);
                if fallback.is_some() {
                    winning_route = TextureDecoder::Pixbuf;
                }
                fallback
            } else {
                None
            }
        })
        .or_else(|| {
            let fallback = decode_image_texture(path);
            if fallback.is_some() {
                winning_route = TextureDecoder::ImageFallback;
            }
            fallback
        });

    let elapsed_ms = started.elapsed().as_millis();
    match &result {
        Some(texture) => {
            if winning_route == decoder {
                log::debug!(
                    "Decoded {} via {:?} in {}ms ({}x{})",
                    path.display(),
                    decoder,
                    elapsed_ms,
                    texture.width(),
                    texture.height(),
                );
            } else {
                log::debug!(
                    "Decoded {} via {:?} fallback (primary {:?} failed) in {}ms ({}x{})",
                    path.display(),
                    winning_route,
                    decoder,
                    elapsed_ms,
                    texture.width(),
                    texture.height(),
                );
            }
        }
        None => log::warn!(
            "All decoders rejected {} (route {:?}) after {}ms",
            path.display(),
            decoder,
            elapsed_ms,
        ),
    }
    result
}

fn texture_decoder_for_path(path: &std::path::Path) -> TextureDecoder {
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some(ext) if crate::media_kinds::is_raw_ext(ext) => TextureDecoder::Raw,
        Some("heic" | "heif" | "hif" | "avif") => TextureDecoder::Heif,
        Some("jxl") => TextureDecoder::JpegXl,
        Some("svg" | "svgz") => TextureDecoder::Svg,
        Some("jpe" | "jpeg" | "jpg" | "insp") => TextureDecoder::Jpeg,
        Some("webp") => TextureDecoder::Webp,
        Some("jp2") => TextureDecoder::Jpeg2k,
        Some("psd") => TextureDecoder::Psd,
        Some("bmp" | "gif" | "png" | "tif" | "tiff") => TextureDecoder::Pixbuf,
        _ => TextureDecoder::ImageFallback,
    }
}

fn memory_texture(
    width: u32,
    height: u32,
    format: gdk4::MemoryFormat,
    pixels: Vec<u8>,
    stride: usize,
) -> Option<gdk4::Texture> {
    let width = i32::try_from(width).ok()?;
    let height = i32::try_from(height).ok()?;
    let bytes = glib::Bytes::from_owned(pixels);
    let texture = gdk4::MemoryTexture::new(width, height, format, &bytes, stride);
    Some(texture.upcast::<gdk4::Texture>())
}

/// Max RAW dimension; full-sensor demosaic OOMs on 24+ MP files.
const RAW_MAX_DIMENSION: usize = 4096;

/// Thumbnail-targeted RAW decode: embedded JPEG first, full demosaic only as
/// last resort. Ignores `RAW_FULL_DECODE` (which is for lightbox quality, not
/// 256-px grid tiles -- full sensor data would just be scaled away).
pub(super) fn decode_raw_thumbnail_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    if let Some(tex) = extract_libraw_thumb(path) {
        return Some(tex);
    }
    log::debug!(
        "No embedded preview in {}; thumbnail falling back to full decode",
        path.display()
    );
    if let Some(tex) = decode_libraw_texture(path) {
        return Some(tex);
    }
    // imagepipe pure-Rust fallback -- wrapped in catch_unwind because
    // rawloader panics on OOB slice access for some malformed inputs.
    let path_for_panic = path.to_path_buf();
    let imagepipe_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        imagepipe::simple_decode_8bit(path, RAW_MAX_DIMENSION, RAW_MAX_DIMENSION)
    }));
    match imagepipe_result {
        Ok(Ok(image)) => memory_texture(
            image.width.try_into().ok()?,
            image.height.try_into().ok()?,
            gdk4::MemoryFormat::R8g8b8,
            image.data,
            image.width.checked_mul(3)?,
        ),
        Ok(Err(err)) => {
            log::debug!(
                "imagepipe thumbnail fallback also failed for {}: {}",
                path.display(),
                err
            );
            None
        }
        Err(_) => {
            log::warn!(
                "imagepipe thumbnail fallback panicked for {}",
                path_for_panic.display()
            );
            None
        }
    }
}

fn decode_raw_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let full_decode = RAW_FULL_DECODE.load(std::sync::atomic::Ordering::Relaxed);
    if full_decode {
        // Full demosaic via libraw (slow, highest quality) with optional cache.
        if let Some(texture) = decode_libraw_texture(path) {
            return Some(texture);
        }
        // imagepipe pure-Rust fallback — wrapped in catch_unwind because
        // rawloader panics on OOB slice access for some malformed inputs.
        let path_for_panic = path.to_path_buf();
        let imagepipe_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            imagepipe::simple_decode_8bit(path, RAW_MAX_DIMENSION, RAW_MAX_DIMENSION)
        }));
        match imagepipe_result {
            Ok(Ok(image)) => memory_texture(
                image.width.try_into().ok()?,
                image.height.try_into().ok()?,
                gdk4::MemoryFormat::R8g8b8,
                image.data,
                image.width.checked_mul(3)?,
            ),
            Ok(Err(err)) => {
                log::debug!(
                    "imagepipe RAW fallback also failed for {}: {}",
                    path.display(),
                    err
                );
                None
            }
            Err(_) => {
                log::warn!(
                    "imagepipe RAW fallback panicked for {}",
                    path_for_panic.display()
                );
                None
            }
        }
    } else {
        // Fast path: extract the embedded camera JPEG preview.
        if let Some(texture) = extract_libraw_thumb(path) {
            return Some(texture);
        }
        // Fallback to full decode if no embedded thumbnail is available.
        log::debug!(
            "No embedded preview in {}; falling back to full decode",
            path.display()
        );
        decode_libraw_texture(path)
    }
}

/// Return the directory used for the on-disk RAW decode cache.
fn raw_decode_cache_dir() -> std::path::PathBuf {
    crate::profile::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp").join(crate::profile::dir_segment()))
        .join("raw_decode")
}

/// Build a stable cache filename for a RAW file based on its canonical path,
/// last-modified timestamp (seconds), and byte length.  Any change to the file
/// on disk produces a different key so stale pixels are never served.
fn raw_decode_cache_key(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let size = meta.len();
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    mtime.hash(&mut hasher);
    size.hash(&mut hasher);
    Some(format!("{:016x}.png", hasher.finish()))
}

/// Attempt to load a previously cached demosaiced texture from disk.
/// Returns `None` if the cache entry is absent, stale, or unreadable.
fn read_raw_decode_cache(path: &std::path::Path) -> Option<gdk4::Texture> {
    let key = raw_decode_cache_key(path)?;
    let cache_file = raw_decode_cache_dir().join(&key);
    if !cache_file.exists() {
        return None;
    }
    match gdk4::Texture::from_filename(&cache_file) {
        Ok(texture) => {
            log::debug!("RAW decode cache hit for {}", path.display());
            Some(texture)
        }
        Err(err) => {
            log::debug!(
                "RAW decode cache read failed for {}: {}",
                path.display(),
                err
            );
            // Remove the corrupt entry so the next decode replaces it cleanly.
            let _ = std::fs::remove_file(&cache_file);
            None
        }
    }
}

/// Persist a successfully demosaiced RAW texture to the disk cache so future
/// opens can skip the expensive libraw pipeline entirely.
fn write_raw_decode_cache(path: &std::path::Path, texture: &gdk4::Texture) {
    let Some(key) = raw_decode_cache_key(path) else {
        return;
    };
    let cache_dir = raw_decode_cache_dir();
    if let Err(err) = std::fs::create_dir_all(&cache_dir) {
        log::debug!("RAW decode cache dir create failed: {}", err);
        return;
    }
    let cache_file = cache_dir.join(&key);
    // Save as PNG via GDK so we roundtrip losslessly through the same loader.
    if let Err(err) = texture.save_to_png(&cache_file) {
        log::debug!(
            "RAW decode cache write failed for {}: {}",
            path.display(),
            err
        );
    } else {
        log::debug!("RAW decode cache written for {}", path.display());
    }
}
/// Minimum size for an embedded preview to be considered -- tiny thumbnails
/// (EXIF 160x120 stubs) are almost always useless and should be skipped in
/// favour of the full-resolution preview or the libraw fallback.
const MIN_EMBEDDED_JPEG_SIZE: usize = 4096;

/// True if the JPEG at `bytes[start..end]` uses SOF3 (lossless JPEG).
/// RAW files wrap compressed Bayer data in SOF3 containers -- these are
/// never renderable and should be skipped by the preview scanner.
fn is_lossless_jpeg(bytes: &[u8], start: usize, end: usize) -> bool {
    // Walk the marker structure from just after SOI (FF D8) looking for
    // a SOF marker. SOF markers are C0-CF excluding C4 (DHT), C8
    // (reserved), and CC (DAC).
    let mut pos = start + 2;
    while pos + 3 < end {
        if bytes[pos] != 0xFF {
            return false;
        }
        let marker = bytes[pos + 1];
        // Skip fill bytes and parameterless markers.
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        // SOF marker range: C0-CF except C4, C8, CC.
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            return marker == 0xC3;
        }
        // SOS -- no SOF found before entropy data; not lossless.
        if marker == 0xDA {
            return false;
        }
        // Variable-length segment: skip by declared length.
        if pos + 3 >= end {
            return false;
        }
        let seg_len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if seg_len < 2 || pos + 2 + seg_len > end {
            return false;
        }
        pos = pos + 2 + seg_len;
    }
    false
}

/// Return the largest renderable JPEG embedded in the file, or None.
/// Uses structure-aware marker walking; skips lossless SOF3 payloads
/// (compressed Bayer data) and payloads below `MIN_EMBEDDED_JPEG_SIZE`.
fn extract_largest_embedded_jpeg(path: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let mut best: Option<(usize, usize)> = None;
    let len = bytes.len();
    let mut i = 0;

    while i + 3 < len {
        // Look for SOI: FF D8 FF xx (xx != 00).
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && bytes[i + 2] == 0xFF && bytes[i + 3] != 0x00
        {
            if let Some(end) = find_jpeg_end(&bytes, i) {
                let payload_len = end - i;
                if payload_len >= MIN_EMBEDDED_JPEG_SIZE
                    && !is_lossless_jpeg(&bytes, i, end)
                    && best.is_none_or(|(_, l)| payload_len > l)
                {
                    best = Some((i, payload_len));
                }
                i = end;
                continue;
            }
            // No end found at all -- skip past this SOI.
            i += 2;
            continue;
        }
        i += 1;
    }

    best.map(|(start, l)| bytes[start..start + l].to_vec())
}

/// Walk the JPEG marker structure starting at the SOI at `bytes[start]` and
/// return the byte offset just past the EOI (or the end of the buffer if the
/// file omits the trailing EOI).
///
/// The scanner reads marker segments by their declared lengths, then
/// byte-scans only through the entropy-coded section after SOS where
/// FF 00 (byte-stuffing) and FF D0-D7 (restart markers) are skipped.
fn find_jpeg_end(bytes: &[u8], start: usize) -> Option<usize> {
    let len = bytes.len();
    // Skip past SOI (FF D8).
    let mut pos = start + 2;

    loop {
        // Find the next 0xFF marker prefix.
        while pos < len && bytes[pos] != 0xFF {
            pos += 1;
        }
        if pos + 1 >= len {
            // Reached EOF without an explicit EOI. Accept the whole tail
            // as the JPEG payload — some RAW formats omit the trailing D9.
            return Some(len);
        }

        // Skip any padding FFs (JPEG spec allows fill bytes before markers).
        while pos + 1 < len && bytes[pos + 1] == 0xFF {
            pos += 1;
        }
        if pos + 1 >= len {
            return Some(len);
        }

        let marker = bytes[pos + 1];
        match marker {
            // FF 00: byte-stuffed escape inside entropy data — not a marker.
            0x00 => {
                pos += 2;
            }
            // EOI: end of this JPEG stream.
            0xD9 => {
                return Some(pos + 2);
            }
            // SOI: nested JPEG start — abort this stream (the outer loop
            // will pick it up as a new candidate).
            0xD8 => {
                return Some(pos);
            }
            // RST0-RST7: restart markers (no payload).
            0xD0..=0xD7 => {
                pos += 2;
            }
            // SOS (Start of Scan): read the segment header, then enter
            // the entropy-coded section where only byte-stuffing and
            // restart markers are valid FF-sequences.
            0xDA => {
                if pos + 3 >= len {
                    return Some(len);
                }
                let seg_len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
                if seg_len < 2 {
                    return Some(len);
                }
                pos = pos + 2 + seg_len;
                // Now inside entropy data: scan for the next real marker.
                // FF 00 and FF D0-D7 are part of the stream; anything
                // else is a marker that ends the entropy section.
                while pos + 1 < len {
                    if bytes[pos] == 0xFF {
                        let next = bytes[pos + 1];
                        if next == 0x00 || (0xD0..=0xD7).contains(&next) {
                            // Byte-stuffed escape or restart marker.
                            pos += 2;
                            continue;
                        }
                        // Real marker found — break out to the outer loop
                        // which will classify it (EOI, another SOS, etc.).
                        break;
                    }
                    pos += 1;
                }
                if pos + 1 >= len {
                    return Some(len);
                }
            }
            // All other markers: fixed-length header markers (TEM=0x01)
            // and variable-length segments (APP0-APPn, DQT, DHT, SOF, COM,
            // etc.) — skip by declared segment length.
            0x01 => {
                pos += 2;
            }
            _ => {
                if pos + 3 >= len {
                    return Some(len);
                }
                let seg_len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
                if seg_len < 2 || pos + 2 + seg_len > len {
                    // Malformed segment length — accept what we have.
                    return Some(len);
                }
                pos = pos + 2 + seg_len;
            }
        }
    }
}

/// Read EXIF orientation (tag 0x0112) from JPEG bytes. Returns 1..=8 or None.
/// Hand-rolled because `Pixbuf::apply_embedded_orientation` silently drops
/// EXIF on some RAW-embedded JPEGs (truncated/oversize APP1 segments).
fn read_jpeg_exif_orientation(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xD8 || marker == 0xD9 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if seg_len < 2 || i + 2 + seg_len > bytes.len() {
            return None;
        }
        let seg = &bytes[i + 4..i + 2 + seg_len];
        if marker == 0xE1 && seg.len() >= 6 && &seg[..6] == b"Exif\0\0" {
            return parse_tiff_orientation(&seg[6..]);
        }
        i += 2 + seg_len;
    }
    None
}

/// Parse TIFF IFD0 starting at the TIFF header, return orientation if present.
fn parse_tiff_orientation(tiff: &[u8]) -> Option<u8> {
    if tiff.len() < 8 {
        return None;
    }
    let little = match &tiff[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |p: &[u8]| -> u16 {
        if little {
            u16::from_le_bytes([p[0], p[1]])
        } else {
            u16::from_be_bytes([p[0], p[1]])
        }
    };
    let read_u32 = |p: &[u8]| -> u32 {
        if little {
            u32::from_le_bytes([p[0], p[1], p[2], p[3]])
        } else {
            u32::from_be_bytes([p[0], p[1], p[2], p[3]])
        }
    };
    if read_u16(&tiff[2..4]) != 0x002A {
        return None;
    }
    let ifd0 = read_u32(&tiff[4..8]) as usize;
    if ifd0 + 2 > tiff.len() {
        return None;
    }
    let count = read_u16(&tiff[ifd0..ifd0 + 2]) as usize;
    for n in 0..count {
        let off = ifd0 + 2 + n * 12;
        if off + 12 > tiff.len() {
            return None;
        }
        if read_u16(&tiff[off..off + 2]) == 0x0112 {
            let v = read_u16(&tiff[off + 8..off + 10]) as u8;
            return (1..=8).contains(&v).then_some(v);
        }
    }
    None
}

/// Apply EXIF orientation (1..=8) to a Pixbuf via rotate + optional flip.
fn apply_exif_orientation(pixbuf: &gtk::gdk_pixbuf::Pixbuf, orient: u8) -> gtk::gdk_pixbuf::Pixbuf {
    use gtk::gdk_pixbuf::PixbufRotation;
    let rotated = match orient {
        3 | 4 => pixbuf.rotate_simple(PixbufRotation::Upsidedown),
        5 | 8 => pixbuf.rotate_simple(PixbufRotation::Counterclockwise),
        6 | 7 => pixbuf.rotate_simple(PixbufRotation::Clockwise),
        _ => Some(pixbuf.clone()),
    }
    .unwrap_or_else(|| pixbuf.clone());
    match orient {
        2 | 4 | 5 | 7 => rotated.flip(true).unwrap_or(rotated),
        _ => rotated,
    }
}

/// Check whether a thumbnail has already been rotated to display orientation
/// by the camera firmware. Compares the thumbnail's aspect ratio to the raw
/// sensor's: if a 90-degree flip is needed (flip=5 or flip=6) but the
/// thumbnail is already in the rotated aspect (portrait when sensor is
/// landscape, or vice versa), the thumbnail is pre-rotated.
fn is_thumbnail_prerotated(
    thumb_w: i32,
    thumb_h: i32,
    sensor_w: u32,
    sensor_h: u32,
    flip: std::ffi::c_int,
) -> bool {
    // Only relevant for 90-degree rotations.
    if flip != 5 && flip != 6 {
        return false;
    }
    let sensor_landscape = sensor_w >= sensor_h;
    let thumb_landscape = thumb_w >= thumb_h;
    // After a 90-degree rotation the aspect flips. If the thumbnail already
    // has the flipped aspect (portrait when sensor is landscape), the camera
    // stored it pre-rotated.
    sensor_landscape != thumb_landscape
}

/// Decode JPEG bytes to a texture, applying EXIF orientation or libraw flip.
///
/// The `flip` and `sensor_dims` parameters come from the RAW container (libraw).
/// When the decoded JPEG's aspect ratio already matches the rotated sensor
/// orientation, the container flip is skipped (the camera pre-rotated the
/// preview). When the JPEG carries its own EXIF orientation > 1, that value
/// is trusted unconditionally.
fn jpeg_bytes_to_oriented_texture(
    bytes: &[u8],
    flip: i32,
    sensor_dims: (u32, u32),
) -> Option<gdk4::Texture> {
    let stream = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes));
    let raw_pixbuf =
        gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE).ok()?;
    let exif_orient = read_jpeg_exif_orientation(bytes);
    let oriented = match exif_orient {
        // Embedded JPEG carries a meaningful rotation -- trust it.
        Some(o) if o > 1 => apply_exif_orientation(&raw_pixbuf, o),
        // EXIF says identity (1) or no EXIF tag at all:
        // apply the container flip only if the thumbnail is NOT pre-rotated.
        _ => {
            if is_thumbnail_prerotated(
                raw_pixbuf.width(),
                raw_pixbuf.height(),
                sensor_dims.0,
                sensor_dims.1,
                flip,
            ) {
                raw_pixbuf
            } else {
                apply_libraw_flip(&raw_pixbuf, flip)
            }
        }
    };
    pixbuf_to_texture(&oriented)
}

/// Embedded RAW preview: SOI-scan for the largest JPEG (primary), fall back
/// to libraw's bitmap/JPEG thumbnail for files that store the preview as
/// TIFF strips (Samsung DNG, OnePlus DNG, etc.). Full demosaic path untouched.
fn extract_libraw_thumb(path: &std::path::Path) -> Option<gdk4::Texture> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: Interfacing with the external LibRaw C API via FFI bindings.
    // We check that the context is initialized successfully (non-null),
    // and wrap the raw pointers in custom RAII guards (LibrawHandle, MemImage)
    // to guarantee that resources are cleaned up correctly when they go out of scope.
    unsafe {
        let lr = libraw_sys::libraw_init(0);
        if lr.is_null() {
            return None;
        }
        struct LibrawHandle(*mut libraw_sys::libraw_data_t);
        impl Drop for LibrawHandle {
            fn drop(&mut self) {
                // SAFETY: The handle is verified to be non-null and is safe to close.
                unsafe { libraw_sys::libraw_close(self.0) };
            }
        }
        let _guard = LibrawHandle(lr);
        if libraw_sys::libraw_open_file(lr, c_path.as_ptr()) != 0 {
            return None;
        }

        // Container orientation and sensor dimensions — used to decide
        // whether a thumbnail is pre-rotated by comparing aspect ratios.
        let flip = (*lr).sizes.flip;
        let sensor_dims = ((*lr).sizes.width as u32, (*lr).sizes.height as u32);

        // PRIMARY: scan the file for the largest embedded JPEG and decode it.
        if let Some(scan) = extract_largest_embedded_jpeg(path) {
            let scan_len = scan.len();
            if let Some(texture) = jpeg_bytes_to_oriented_texture(&scan, flip, sensor_dims) {
                log::debug!(
                    "Extracted embedded JPEG preview ({} bytes via SOI-scan, flip={}) from {}",
                    scan_len,
                    flip,
                    path.display()
                );
                return Some(texture);
            }
            log::debug!(
                "SOI-scanned JPEG ({} bytes) failed to decode for {}; falling back to libraw",
                scan_len,
                path.display()
            );
        }

        // FALLBACK: libraw thumb — handles bitmap previews (Samsung/OnePlus
        // DNG store uncompressed TIFF strips) and non-contiguous JPEG-in-strip.
        if libraw_sys::libraw_unpack_thumb(lr) != 0 {
            log::debug!("No embedded thumbnail in {}", path.display());
            return None;
        }
        let mut errcode = 0i32;
        let img = libraw_sys::libraw_dcraw_make_mem_thumb(lr, &mut errcode);
        if img.is_null() || errcode != 0 {
            log::debug!(
                "libraw_dcraw_make_mem_thumb failed ({}) for {}",
                errcode,
                path.display()
            );
            return None;
        }
        struct MemImage(*mut libraw_sys::libraw_processed_image_t);
        impl Drop for MemImage {
            fn drop(&mut self) {
                // SAFETY: The image buffer is verified to be non-null and is safe to clear.
                unsafe { libraw_sys::libraw_dcraw_clear_mem(self.0) };
            }
        }
        let _img_guard = MemImage(img);
        let data_size = (*img).data_size as usize;
        // SAFETY: The image buffer is owned by the `img` memory structure, and its layout is
        // guaranteed to have at least `data_size` bytes. The slice does not outlive its owner
        // (as it is dropped prior to `_img_guard` and its contents are immediately copied).
        let bytes = std::slice::from_raw_parts((*img).data.as_ptr(), data_size);

        if (*img).type_ == libraw_sys::LibRaw_image_formats_LIBRAW_IMAGE_JPEG {
            let texture = jpeg_bytes_to_oriented_texture(bytes, flip, sensor_dims);
            if texture.is_some() {
                log::debug!(
                    "Extracted embedded JPEG preview ({} bytes via libraw, flip={}) from {}",
                    data_size,
                    flip,
                    path.display()
                );
            } else {
                log::debug!(
                    "libraw JPEG thumb failed to decode via pixbuf for {}",
                    path.display()
                );
            }
            texture
        } else {
            // Bitmap thumb (Samsung/OnePlus DNG): build Pixbuf, apply flip
            // only if the thumbnail is NOT already pre-rotated.
            let width = (*img).width as i32;
            let height = (*img).height as i32;
            let colors = (*img).colors as i32;
            if colors != 3 {
                log::debug!(
                    "Embedded thumbnail has {} channels for {} -- skipping",
                    colors,
                    path.display()
                );
                return None;
            }
            let row_stride = width.checked_mul(colors)?;
            let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_mut_slice(
                bytes.to_vec(),
                gtk::gdk_pixbuf::Colorspace::Rgb,
                false,
                8,
                width,
                height,
                row_stride,
            );
            let prerotated =
                is_thumbnail_prerotated(width, height, sensor_dims.0, sensor_dims.1, flip);
            let oriented = if prerotated {
                pixbuf
            } else {
                apply_libraw_flip(&pixbuf, flip)
            };
            let texture = pixbuf_to_texture(&oriented);
            if texture.is_some() {
                log::debug!(
                    "Extracted embedded bitmap preview (flip={}, prerotated={}) from {}",
                    flip,
                    prerotated,
                    path.display()
                );
            }
            texture
        }
    }
}

/// Convert a `Pixbuf` to a `gdk4::Texture` (same logic as `decode_pixbuf_texture`).
fn pixbuf_to_texture(pixbuf: &gtk::gdk_pixbuf::Pixbuf) -> Option<gdk4::Texture> {
    let format = if pixbuf.has_alpha() {
        gdk4::MemoryFormat::R8g8b8a8
    } else {
        gdk4::MemoryFormat::R8g8b8
    };
    let bytes = pixbuf.read_pixel_bytes();
    let texture = gdk4::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &bytes,
        pixbuf.rowstride() as usize,
    );
    Some(texture.upcast::<gdk4::Texture>())
}

/// Apply libraw `flip` orientation to a `Pixbuf`.
///
/// libraw flip values:
///   0 — no rotation (identity)
///   3 — 180 degrees
///   5 — 90 CCW (270 CW)
///   6 — 90 CW
///
/// Other values (rare) are left unrotated.
fn apply_libraw_flip(
    pixbuf: &gtk::gdk_pixbuf::Pixbuf,
    flip: std::ffi::c_int,
) -> gtk::gdk_pixbuf::Pixbuf {
    use gtk::gdk_pixbuf::PixbufRotation;
    match flip {
        3 => pixbuf
            .rotate_simple(PixbufRotation::Upsidedown)
            .unwrap_or_else(|| pixbuf.clone()),
        5 => pixbuf
            .rotate_simple(PixbufRotation::Counterclockwise)
            .unwrap_or_else(|| pixbuf.clone()),
        6 => pixbuf
            .rotate_simple(PixbufRotation::Clockwise)
            .unwrap_or_else(|| pixbuf.clone()),
        _ => pixbuf.clone(),
    }
}

/// Decode a RAW via vendored libraw with camera white balance + AHD demosaic
/// at full resolution. Output goes through a disk cache keyed by (path, mtime,
/// size) so a re-open hits the cache instead of re-demosaicing.
fn decode_libraw_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let cache_enabled = RAW_CACHE_ENABLED.load(std::sync::atomic::Ordering::Relaxed);
    if cache_enabled && let Some(texture) = read_raw_decode_cache(path) {
        return Some(texture);
    }
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: Interfacing with the external LibRaw C API via FFI bindings.
    // We check that the context is initialized successfully (non-null),
    // and wrap the raw pointers in custom RAII guards (LibrawHandle, MemImage)
    // to guarantee that resources are cleaned up correctly when they go out of scope.
    unsafe {
        let lr = libraw_sys::libraw_init(0);
        if lr.is_null() {
            log::debug!("libraw_init returned null for {}", path.display());
            return None;
        }
        struct LibrawHandle(*mut libraw_sys::libraw_data_t);
        impl Drop for LibrawHandle {
            fn drop(&mut self) {
                // SAFETY: The handle is verified to be non-null and is safe to close.
                unsafe { libraw_sys::libraw_close(self.0) };
            }
        }
        let _guard = LibrawHandle(lr);
        (*lr).params.use_camera_wb = 1;
        (*lr).params.output_bps = 8;
        (*lr).params.output_color = 1; // sRGB
        (*lr).params.user_qual = 3; // AHD demosaic (highest quality)
        let rc = libraw_sys::libraw_open_file(lr, c_path.as_ptr());
        if rc != 0 {
            log::debug!("libraw_open_file failed ({}) for {}", rc, path.display());
            return None;
        }
        let rc = libraw_sys::libraw_unpack(lr);
        if rc != 0 {
            log::debug!("libraw_unpack failed ({}) for {}", rc, path.display());
            return None;
        }
        let rc = libraw_sys::libraw_dcraw_process(lr);
        if rc != 0 {
            log::debug!(
                "libraw_dcraw_process failed ({}) for {}",
                rc,
                path.display()
            );
            return None;
        }
        let mut errcode = 0i32;
        let img = libraw_sys::libraw_dcraw_make_mem_image(lr, &mut errcode);
        if img.is_null() || errcode != 0 {
            log::debug!(
                "libraw_dcraw_make_mem_image failed ({}) for {}",
                errcode,
                path.display()
            );
            return None;
        }
        struct MemImage(*mut libraw_sys::libraw_processed_image_t);
        impl Drop for MemImage {
            fn drop(&mut self) {
                // SAFETY: The image buffer is verified to be non-null and is safe to clear.
                unsafe { libraw_sys::libraw_dcraw_clear_mem(self.0) };
            }
        }
        let _img_guard = MemImage(img);
        let width = (*img).width as u32;
        let height = (*img).height as u32;
        let colors = (*img).colors;
        let data_size = (*img).data_size as usize;
        if colors != 3 {
            log::debug!(
                "libraw produced {}-channel output for {} — skipping",
                colors,
                path.display()
            );
            return None;
        }
        // SAFETY: The image buffer is owned by the `img` memory structure, and its layout is
        // guaranteed to have at least `data_size` bytes. The slice does not outlive its owner
        // (as it is dropped prior to `_img_guard` and its contents are immediately copied).
        let pixels = std::slice::from_raw_parts((*img).data.as_ptr(), data_size).to_vec();
        let texture = memory_texture(
            width,
            height,
            gdk4::MemoryFormat::R8g8b8,
            pixels,
            usize::try_from(width).ok()?.checked_mul(3)?,
        );
        if let Some(ref tex) = texture
            && cache_enabled
        {
            let path_clone = path.to_path_buf();
            let tex_clone = tex.clone();
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::task::spawn_blocking(move || {
                    write_raw_decode_cache(&path_clone, &tex_clone);
                });
            } else {
                std::thread::spawn(move || {
                    write_raw_decode_cache(&path_clone, &tex_clone);
                });
            }
        }
        texture
    }
}

fn decode_heif_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

    let libheif = LibHeif::new();
    let encoded = std::fs::read(path).ok()?;
    let context = HeifContext::read_from_bytes(&encoded).ok()?;
    let handle = context.primary_image_handle().ok()?;
    let image = libheif
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .ok()?;
    let plane = image.planes().interleaved?;
    memory_texture(
        plane.width,
        plane.height,
        gdk4::MemoryFormat::R8g8b8a8,
        plane.data.to_vec(),
        plane.stride,
    )
}

fn decode_jpegxl_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) => {
            log::warn!("JXL read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let image = match jxl_oxide::JxlImage::builder().read(file) {
        Ok(image) => image,
        Err(err) => {
            log::warn!("JXL parse failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let render = match image.render_frame(0) {
        Ok(render) => render,
        Err(err) => {
            log::warn!("JXL render failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels();
    let pixel_count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let mut buf = vec![0u8; pixel_count.checked_mul(channels as usize)?];
    stream.write_to_buffer::<u8>(&mut buf);
    // 1 = grayscale, 2 = grayscale + alpha — expand to RGB/RGBA so GDK can
    // upload the texture (GDK doesn't have a 1-channel grayscale format).
    let (format, bpp, buf) = match channels {
        1 => {
            let mut rgb = Vec::with_capacity(pixel_count.checked_mul(3)?);
            for v in buf {
                rgb.extend_from_slice(&[v, v, v]);
            }
            (gdk4::MemoryFormat::R8g8b8, 3usize, rgb)
        }
        2 => {
            let mut rgba = Vec::with_capacity(pixel_count.checked_mul(4)?);
            for chunk in buf.chunks_exact(2) {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            (gdk4::MemoryFormat::R8g8b8a8, 4usize, rgba)
        }
        3 => (gdk4::MemoryFormat::R8g8b8, 3usize, buf),
        4 => (gdk4::MemoryFormat::R8g8b8a8, 4usize, buf),
        _ => {
            log::warn!(
                "JXL unsupported channel count {} for {}",
                channels,
                path.display()
            );
            return None;
        }
    };
    memory_texture(
        width,
        height,
        format,
        buf,
        usize::try_from(width).ok()?.checked_mul(bpp)?,
    )
}

fn decode_image_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let reader = match image::ImageReader::open(path) {
        Ok(reader) => reader,
        Err(err) => {
            log::debug!("image-rs open failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let reader = match reader.with_guessed_format() {
        Ok(reader) => reader,
        Err(err) => {
            log::debug!(
                "image-rs format probe failed for {}: {}",
                path.display(),
                err
            );
            return None;
        }
    };
    match reader.decode() {
        Ok(image) => dynamic_image_texture(image),
        Err(err) => {
            log::debug!("image-rs decode failed for {}: {}", path.display(), err);
            None
        }
    }
}

fn dynamic_image_texture(image: image::DynamicImage) -> Option<gdk4::Texture> {
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    memory_texture(
        width,
        height,
        gdk4::MemoryFormat::R8g8b8a8,
        rgba.into_raw(),
        usize::try_from(width).ok()?.checked_mul(4)?,
    )
}

fn decode_pixbuf_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let raw = match gtk::gdk_pixbuf::Pixbuf::from_file(path) {
        Ok(pixbuf) => pixbuf,
        Err(err) => {
            log::debug!("pixbuf decode failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let pixbuf = raw.apply_embedded_orientation().unwrap_or(raw);
    let format = if pixbuf.has_alpha() {
        gdk4::MemoryFormat::R8g8b8a8
    } else {
        gdk4::MemoryFormat::R8g8b8
    };
    let bytes = pixbuf.read_pixel_bytes();
    let texture = gdk4::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &bytes,
        pixbuf.rowstride() as usize,
    );
    Some(texture.upcast::<gdk4::Texture>())
}

/// Maximum SVG raster dimension; SVGs are vector and could otherwise render at
/// any size. 4096 matches the RAW cap and keeps memory bounded.
const SVG_MAX_DIMENSION: u32 = 4096;

fn decode_svg_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("SVG read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let opt = resvg::usvg::Options::default();
    let prepared = inject_missing_xmlns(&bytes);
    let tree = match resvg::usvg::Tree::from_data(&prepared, &opt) {
        Ok(tree) => tree,
        Err(err) => {
            log::warn!("SVG parse failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let svg_size = tree.size();
    let svg_w = svg_size.width().max(1.0);
    let svg_h = svg_size.height().max(1.0);
    let scale = (SVG_MAX_DIMENSION as f32 / svg_w)
        .min(SVG_MAX_DIMENSION as f32 / svg_h)
        .min(1.0);
    let width = (svg_w * scale).ceil() as u32;
    let height = (svg_h * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    memory_texture(
        width,
        height,
        gdk4::MemoryFormat::R8g8b8a8,
        pixmap.take(),
        usize::try_from(width).ok()?.checked_mul(4)?,
    )
}

fn decode_jpeg_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("JPEG read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let orient = read_jpeg_exif_orientation(&bytes);
    match turbojpeg::decompress(&bytes, turbojpeg::PixelFormat::RGB) {
        Ok(image) => {
            let width: i32 = image.width.try_into().ok()?;
            let height: i32 = image.height.try_into().ok()?;
            let raw_pixbuf = gtk::gdk_pixbuf::Pixbuf::from_mut_slice(
                image.pixels,
                gtk::gdk_pixbuf::Colorspace::Rgb,
                false,
                8,
                width,
                height,
                image.pitch as i32,
            );
            let oriented = match orient {
                Some(o) if o != 1 => apply_exif_orientation(&raw_pixbuf, o),
                _ => raw_pixbuf,
            };
            pixbuf_to_texture(&oriented)
        }
        Err(err) => {
            log::debug!("turbojpeg decode failed for {}: {}", path.display(), err);
            None
        }
    }
}

fn decode_webp_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("WebP read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let decoder = webp::Decoder::new(&bytes);
    let image = match decoder.decode() {
        Some(image) => image,
        None => {
            log::debug!("libwebp decode failed for {}", path.display());
            return None;
        }
    };
    let format = if image.is_alpha() {
        gdk4::MemoryFormat::R8g8b8a8
    } else {
        gdk4::MemoryFormat::R8g8b8
    };
    let bpp = if image.is_alpha() { 4 } else { 3 };
    let width = image.width();
    let height = image.height();
    memory_texture(
        width,
        height,
        format,
        image.to_vec(),
        usize::try_from(width).ok()?.checked_mul(bpp)?,
    )
}

fn decode_jpeg2k_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("JP2 read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let image = match jpeg2k::Image::from_bytes(&bytes) {
        Ok(image) => image,
        Err(err) => {
            log::warn!("JP2 parse failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let pixels = match image.get_pixels(Some(255)) {
        Ok(pixels) => pixels,
        Err(err) => {
            log::warn!("JP2 pixel extract failed for {}: {}", path.display(), err);
            return None;
        }
    };
    use jpeg2k::ImagePixelData;
    let (format, bytes, bpp) = match pixels.data {
        ImagePixelData::Rgb8(data) => (gdk4::MemoryFormat::R8g8b8, data, 3),
        ImagePixelData::Rgba8(data) => (gdk4::MemoryFormat::R8g8b8a8, data, 4),
        ImagePixelData::L8(data) => {
            let mut rgb = Vec::with_capacity(data.len() * 3);
            for v in data {
                rgb.extend_from_slice(&[v, v, v]);
            }
            (gdk4::MemoryFormat::R8g8b8, rgb, 3)
        }
        ImagePixelData::La8(data) => {
            let mut rgba = Vec::with_capacity(data.len() * 2);
            for chunk in data.chunks_exact(2) {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            (gdk4::MemoryFormat::R8g8b8a8, rgba, 4)
        }
        _ => {
            log::warn!("JP2 unsupported 16-bit pixel layout for {}", path.display());
            return None;
        }
    };
    memory_texture(
        pixels.width,
        pixels.height,
        format,
        bytes,
        usize::try_from(pixels.width).ok()?.checked_mul(bpp)?,
    )
}

fn decode_psd_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = std::fs::read(path)
        .map_err(|err| log::warn!("PSD read failed for {}: {}", path.display(), err))
        .ok()?;
    let psd = psd::Psd::from_bytes(&bytes)
        .map_err(|err| log::warn!("PSD parse failed for {}: {:?}", path.display(), err))
        .ok()?;
    let width = psd.width();
    let height = psd.height();
    memory_texture(
        width,
        height,
        gdk4::MemoryFormat::R8g8b8a8,
        psd.rgba(),
        usize::try_from(width).ok()?.checked_mul(4)?,
    )
}

/// Inject placeholder `xmlns:` declarations for any prefix used but not
/// declared (e.g. `c2pa:` in C2PA-signed SVGs) so usvg's strict XML
/// parser accepts the document.
fn inject_missing_xmlns(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let mut declared: std::collections::HashSet<String> = ["xml", "xmlns", "xlink"]
        .into_iter()
        .map(String::from)
        .collect();
    let bytes_str = text.as_bytes();
    let mut i = 0;
    while let Some(rel) = text[i..].find("xmlns:") {
        let start = i + rel + 6;
        let mut end = start;
        while end < bytes_str.len() {
            let b = bytes_str[end];
            if b == b'=' || b == b' ' || b == b'\t' || b == b'\n' || b == b'/' || b == b'>' {
                break;
            }
            end += 1;
        }
        if end > start {
            declared.insert(text[start..end].to_string());
        }
        i = end;
    }
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut j = 0;
    while j < bytes_str.len() {
        let b = bytes_str[j];
        let starts_token = b == b'<' || b == b' ' || b == b'\t' || b == b'\n';
        if starts_token {
            let mut k = j + 1;
            if k < bytes_str.len() && bytes_str[k] == b'/' {
                k += 1;
            }
            let ident_start = k;
            while k < bytes_str.len() {
                let c = bytes_str[k];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                    k += 1;
                } else {
                    break;
                }
            }
            if k > ident_start && k < bytes_str.len() && bytes_str[k] == b':' {
                let prefix = &text[ident_start..k];
                if !declared.contains(prefix) {
                    used.insert(prefix.to_string());
                }
            }
            j = k.max(j + 1);
        } else {
            j += 1;
        }
    }
    if used.is_empty() {
        return bytes.to_vec();
    }
    let Some(svg_tag) = text.find("<svg") else {
        return bytes.to_vec();
    };
    let insert_at = svg_tag + 4;
    let mut decls = String::new();
    for prefix in &used {
        decls.push_str(&format!(
            " xmlns:{prefix}=\"urn:mimick:placeholder:{prefix}\""
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() + decls.len());
    out.extend_from_slice(&bytes[..insert_at]);
    out.extend_from_slice(decls.as_bytes());
    out.extend_from_slice(&bytes[insert_at..]);
    out
}

/// The complete UI widgets state wrapper for the library window interface.
struct LibraryWindowUi {
    ctx: Arc<AppContext>,
    app: libadwaita::Application,
    window: libadwaita::ApplicationWindow,
    nav: libadwaita::NavigationView,
    sidebar: SidebarParts,
    grid: GridViewParts,
    explore: ExploreViewParts,
    albums: AlbumsViewParts,
    content_stack: gtk::Stack,
    error_label: gtk::Label,
    transfer_bar: gtk::Box,
    transfer_progress: gtk::ProgressBar,
    transfer_icon: gtk::Image,
    transfer_label: gtk::Label,
    search_entry: gtk::SearchEntry,
    search_mode: gtk::DropDown,
    sort_mode: gtk::DropDown,
    source_mode: gtk::DropDown,
    source_revealer: gtk::Revealer,
    upload_button: gtk::Button,
    filters_button: gtk::Button,
    timeline_toggle: gtk::ToggleButton,
    timeline_banner: gtk::Label,
    source_mode_suppressed: Cell<bool>,
    sidebar_suppressed: Cell<bool>,
    back_button: gtk::Button,
    select_toggle: gtk::ToggleButton,
    bulk_bar: gtk::Revealer,
    bulk_count_label: gtk::Label,
    album_link_row: libadwaita::ActionRow,
    album_link_button: gtk::Button,
    album_sync_button: gtk::Button,
    last_seen_upload_batch: Cell<u64>,
    narrow: Rc<Cell<bool>>,
    split: libadwaita::OverlaySplitView,
}

/// Build and display the main library window application layout.
pub fn build_library_window(app: &libadwaita::Application, ctx: Arc<AppContext>) {
    style::ensure_registered();
    register_app_icons();

    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Mimick Library")
        .name("mimick-library-window")
        .default_width(1480)
        .default_height(780)
        .width_request(360)
        .height_request(480)
        .build();

    let header = libadwaita::HeaderBar::builder()
        .show_start_title_buttons(true)
        .show_end_title_buttons(true)
        .build();
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle sidebar (F9)")
        .active(true)
        .css_classes(["mimick-pressable"])
        .build();
    let back_button = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Back (Alt+Left)")
        .sensitive(false)
        .css_classes(["mimick-pressable"])
        .build();
    let menu = gtk::gio::Menu::new();
    menu.append(Some("Refresh"), Some("win.refresh"));
    menu.append(Some("Queue Inspector"), Some("win.queue"));
    menu.append(Some("Settings"), Some("win.settings"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Menu")
        .css_classes(["mimick-pressable"])
        .build();
    header.pack_start(&sidebar_toggle);
    header.pack_start(&back_button);
    header.pack_end(&menu_button);
    let select_toggle = gtk::ToggleButton::builder()
        .icon_name("checkbox-symbolic")
        .tooltip_text("Select assets (Esc to exit)")
        .build();

    let toolbar = libadwaita::ToolbarView::builder().build();
    toolbar.add_top_bar(&header);

    let narrow_flag = Rc::new(Cell::new(false));
    let sidebar = build_sidebar();
    let grid = build_grid_view(ctx.clone(), select_toggle.clone(), narrow_flag.clone());
    let explore = build_explore_view();
    let albums = build_albums_view();

    let source_mode_model = gtk::StringList::new(&["Remote", "Local", "Unified"]);
    let source_mode = gtk::DropDown::builder()
        .model(&source_mode_model)
        .selected(0)
        .tooltip_text("Asset source")
        .build();
    let timeline_toggle = gtk::ToggleButton::builder()
        .label("Timeline")
        .tooltip_text("Timeline view (all assets only)")
        .build();

    // Three distinct search dimensions, each routed to a different Immich
    // endpoint shape. Smart and OCR are *separate* fields on the Immich
    // search DTOs (`query` vs `ocr` per the live OpenAPI spec), so we
    // expose them independently rather than collapsing OCR into Smart.
    let search_mode_model = gtk::StringList::new(&["Filename", "Smart Search", "OCR"]);
    let search_mode = gtk::DropDown::builder()
        .model(&search_mode_model)
        .selected(0)
        .tooltip_text(
            "Filename: matches the file name and EXIF metadata.\n\
             Smart: CLIP-based semantic search — natural-language queries against visual scenes \
             (\"sunset beach\", \"birthday cake\", \"invoices\").\n\
             OCR: matches text recognised inside images by Immich's ML pipeline. Faster than \
             Smart since it skips CLIP inference.",
        )
        .build();
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search filenames")
        .hexpand(true)
        .width_chars(6)
        .max_width_chars(18)
        .build();
    let filters_button = gtk::Button::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("Advanced filters (date, location, camera, EXIF)")
        .build();
    let sort_model = gtk::StringList::new(&["Newest", "Filename", "File Type"]);
    let sort_mode = gtk::DropDown::builder()
        .model(&sort_model)
        .selected(0)
        .build();

    let upload_button = gtk::Button::builder()
        .icon_name("document-send-symbolic")
        .tooltip_text("Upload to library")
        .css_classes(["suggested-action", "mimick-pressable"])
        .build();

    // Source mode (Remote/Local/Unified) is meaningful only inside a linked
    // album; the revealer is unhidden by `apply_view_chrome` per source kind.
    let source_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideRight)
        .transition_duration(180)
        .reveal_child(false)
        .build();
    source_revealer.set_child(Some(&source_mode));

    let source_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    source_group.append(&source_revealer);
    source_group.append(&timeline_toggle);

    let search_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    search_group.append(&search_mode);
    search_group.append(&search_entry);
    search_group.append(&filters_button);

    let sort_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    sort_group.append(&sort_mode);
    sort_group.append(&upload_button);

    let controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(8)
        .margin_end(8)
        .build();
    controls.append(&source_group);
    controls.append(&search_group);
    controls.append(&sort_group);

    let timeline_banner = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["mimick-timeline-banner".to_string()])
        .visible(false)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(20)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(12)
        .build();

    let content_stack = gtk::Stack::builder()
        .vexpand(true)
        .hexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(180)
        .build();
    let loading_view = build_loading_view();
    let empty_view = build_status_view(
        "image-x-generic-symbolic",
        "Nothing to show",
        "No assets match the current view",
    );
    let error_view = build_status_view(
        "dialog-warning-symbolic",
        "Library data unavailable",
        "Could not load library assets",
    );
    let error_label = error_view
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("status-view subtitle label");
    content_stack.add_named(&loading_view, Some("loading"));
    content_stack.add_named(&empty_view, Some("empty"));
    content_stack.add_named(&error_view, Some("error"));
    content_stack.add_named(&grid.scrolled, Some("grid"));
    content_stack.add_named(&explore.root, Some("explore"));
    content_stack.add_named(&albums.root, Some("albums"));

    let transfer_progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(vec!["mimick-transfer-progress".to_string()])
        .build();
    let transfer_icon = gtk::Image::builder()
        .icon_size(gtk::IconSize::Normal)
        .css_classes(vec!["dim-label".to_string()])
        .visible(false)
        .build();
    let transfer_label = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .max_width_chars(24)
        .width_chars(12)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(vec!["caption".to_string(), "dim-label".to_string()])
        .build();
    let transfer_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(16)
        .margin_start(12)
        .margin_end(12)
        .css_classes(vec!["mimick-transfer-shell".to_string()])
        .build();
    transfer_bar.append(&transfer_progress);
    transfer_bar.append(&transfer_icon);
    transfer_bar.append(&transfer_label);

    let album_link_row = libadwaita::ActionRow::builder()
        .title("No local folder linked")
        .subtitle("Drop files in the linked folder to sync this album")
        .title_lines(1)
        .subtitle_lines(2)
        .build();
    let album_sync_button = gtk::Button::builder()
        .label("Sync")
        .valign(gtk::Align::Center)
        .css_classes(vec!["suggested-action".to_string()])
        .visible(false)
        .build();
    let album_link_button = gtk::Button::builder()
        .label("Link")
        .valign(gtk::Align::Center)
        .build();
    album_link_row.add_suffix(&album_sync_button);
    album_link_row.add_suffix(&album_link_button);
    let album_link_listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .margin_bottom(4)
        .visible(false)
        .build();
    album_link_listbox.append(&album_link_row);

    let bulk_count_label = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    let bulk_delete = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete selected")
        .css_classes(vec!["destructive-action".to_string()])
        .build();
    let bulk_download = gtk::Button::builder()
        .icon_name("mimick-download-symbolic")
        .tooltip_text("Download selected")
        .build();
    let bulk_clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("Clear selection")
        .css_classes(vec!["flat".to_string()])
        .build();
    let bulk_inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(8)
        .margin_bottom(16)
        .margin_start(12)
        .margin_end(12)
        .css_classes(vec!["toolbar".to_string()])
        .build();
    bulk_inner.append(&bulk_count_label);
    bulk_inner.append(&bulk_clear);
    bulk_inner.append(&bulk_download);
    bulk_inner.append(&bulk_delete);
    let bulk_bar = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(false)
        .child(&bulk_inner)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&controls);
    content.append(&album_link_listbox);
    content.append(&timeline_banner);
    content.append(&content_stack);
    content.append(&bulk_bar);
    content.append(&transfer_bar);

    let split = libadwaita::OverlaySplitView::builder()
        .sidebar(&sidebar.root)
        .content(&content)
        .show_sidebar(true)
        .enable_show_gesture(true)
        .enable_hide_gesture(true)
        .build();
    split
        .bind_property("show-sidebar", &sidebar_toggle, "active")
        .sync_create()
        .bidirectional()
        .build();
    toolbar.set_content(Some(&split));

    let nav = libadwaita::NavigationView::new();
    let root_page = libadwaita::NavigationPage::builder()
        .child(&toolbar)
        .title("Library")
        .can_pop(false)
        .build();
    nav.add(&root_page);
    window.set_content(Some(&nav));

    let breakpoint = libadwaita::Breakpoint::new(
        libadwaita::BreakpointCondition::parse("max-width: 600sp")
            .expect("valid breakpoint condition"),
    );
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    breakpoint.add_setter(&transfer_bar, "visible", Some(&false.to_value()));
    let narrow_apply = narrow_flag.clone();
    let canvas_for_apply = grid.canvas.clone();
    breakpoint.connect_apply(move |_| {
        narrow_apply.set(true);
        canvas_for_apply.set_narrow(true);
    });
    let narrow_unapply = narrow_flag.clone();
    let canvas_for_unapply = grid.canvas.clone();
    breakpoint.connect_unapply(move |_| {
        narrow_unapply.set(false);
        canvas_for_unapply.set_narrow(false);
    });
    window.add_breakpoint(breakpoint);

    let desktop_bp = libadwaita::Breakpoint::new(
        libadwaita::BreakpointCondition::parse("min-width: 600sp")
            .expect("valid breakpoint condition"),
    );
    let window_for_desktop_apply = window.clone();
    desktop_bp.connect_apply(move |_| {
        window_for_desktop_apply.add_css_class("mimick-wide");
    });
    let window_for_desktop_unapply = window.clone();
    desktop_bp.connect_unapply(move |_| {
        window_for_desktop_unapply.remove_css_class("mimick-wide");
    });
    desktop_bp.add_setter(
        &controls,
        "orientation",
        Some(&gtk::Orientation::Horizontal.to_value()),
    );
    desktop_bp.add_setter(&album_sync_button, "label", Some(&"Sync…".to_value()));
    desktop_bp.add_setter(
        &album_link_button,
        "label",
        Some(&"Link folder…".to_value()),
    );
    window.add_breakpoint(desktop_bp);

    // Tablet-width breakpoint: collapse sidebar to overlay before the inline
    // sidebar + controls (~960 px natural) overflow a shrunk desktop window.
    let tablet_bp = libadwaita::Breakpoint::new(
        libadwaita::BreakpointCondition::parse("max-width: 1000sp")
            .expect("valid breakpoint condition"),
    );
    tablet_bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(tablet_bp);

    let f9 = gtk::Shortcut::builder()
        .trigger(&gtk::ShortcutTrigger::parse_string("F9").unwrap())
        .action(&gtk::CallbackAction::new(clone!(
            #[strong]
            split,
            move |_, _| {
                split.set_show_sidebar(!split.shows_sidebar());
                glib::Propagation::Stop
            }
        )))
        .build();
    let shortcut_controller = gtk::ShortcutController::new();
    shortcut_controller.add_shortcut(f9);
    window.add_controller(shortcut_controller);

    let ui = Rc::new(LibraryWindowUi {
        ctx,
        app: app.clone(),
        window: window.clone(),
        nav: nav.clone(),
        sidebar,
        grid,
        explore,
        albums,
        content_stack,
        error_label,
        transfer_bar,
        transfer_progress,
        transfer_icon,
        transfer_label,
        search_entry,
        search_mode,
        sort_mode,
        source_mode,
        source_revealer,
        upload_button: upload_button.clone(),
        filters_button: filters_button.clone(),
        timeline_toggle,
        timeline_banner,
        source_mode_suppressed: Cell::new(false),
        sidebar_suppressed: Cell::new(false),
        back_button: back_button.clone(),
        select_toggle: select_toggle.clone(),
        bulk_bar: bulk_bar.clone(),
        bulk_count_label: bulk_count_label.clone(),
        album_link_row: album_link_row.clone(),
        album_link_button: album_link_button.clone(),
        album_sync_button: album_sync_button.clone(),
        last_seen_upload_batch: Cell::new(0),
        narrow: narrow_flag.clone(),
        split: split.clone(),
    });
    *ui.grid.context_menu_handler.borrow_mut() = Some(Box::new(clone!(
        #[strong]
        ui,
        move |position, x, y| {
            show_asset_context_menu(ui.clone(), &ui.grid.canvas, position, x, y);
        }
    )));

    connect_album_link_row(ui.clone(), album_link_listbox);

    connect_select_mode(ui.clone(), select_toggle.clone());
    connect_bulk_actions(ui.clone(), bulk_delete, bulk_download, bulk_clear);

    connect_sidebar_handlers(ui.clone());
    connect_controls(ui.clone());
    connect_grid_handlers(ui.clone());
    connect_filters_button(ui.clone(), filters_button);

    let close_ctx = ui.ctx.clone();
    window.connect_close_request(move |_| {
        close_ctx.thumbnail_cache.clear_memory();
        glib::Propagation::Proceed
    });

    bootstrap_window(ui);
    window.present();
}

/// Bootstrap initial data loading and background tasks for the library window.
fn bootstrap_window(ui: Rc<LibraryWindowUi>) {
    let initial_request = {
        let mut state = ui.ctx.library_state.lock();
        state.load_initial_source()
    };

    apply_timeline_ui_state(&ui, &initial_request.1);
    load_albums(ui.clone());
    load_status(ui.clone());
    fetch_current_user(ui.clone());
    connect_albums_create(ui.clone());
    spawn_server_ping_loop(ui.clone());
    spawn_transfer_poll_loop(ui.clone());
    load_source_page(ui, initial_request, false);
}

/// Start a periodic server health checking loop updating status icons in the UI.
fn spawn_server_ping_loop(ui: Rc<LibraryWindowUi>) {
    glib::timeout_add_seconds_local(5, move || {
        let ui_for_tick = ui.clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = ui_for_tick.ctx.api_client.check_connection().await;
            let route = ui_for_tick.ctx.api_client.active_route_label().await;
            update_footer(&ui_for_tick, route);
        });
        glib::ControlFlow::Continue
    });
}

/// Start a quick periodic polling loop updating the transfer progress bar UI.
fn spawn_transfer_poll_loop(ui: Rc<LibraryWindowUi>) {
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let completed_batches = ui.ctx.state.lock().completed_upload_batches;
        if completed_batches != ui.last_seen_upload_batch.get() {
            ui.last_seen_upload_batch.set(completed_batches);
            refresh_library_after_mutation(ui.clone(), true);
        }
        update_transfer_ui(&ui);
        glib::ControlFlow::Continue
    });
}

/// Fetch current logged-in API user ID and store it in app context.
fn fetch_current_user(ui: Rc<LibraryWindowUi>) {
    if ui.ctx.current_user_id.lock().is_some() {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_current_user_id().await {
            Ok(id) => {
                *ui.ctx.current_user_id.lock() = Some(id);
            }
            Err(err) => log::warn!("Could not fetch current user id: {}", err),
        }
    });
}

/// Connect action handlers to the album creation widgets.
fn connect_albums_create(ui: Rc<LibraryWindowUi>) {
    ui.albums.create_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| prompt_create_album(ui.clone())
    ));
}

/// Show a popup modal dialog prompting the user to name and create a new album.
fn prompt_create_album(ui: Rc<LibraryWindowUi>) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Create album")
        .body("Choose a name for the new album.")
        .build();
    let entry = gtk::Entry::builder()
        .placeholder_text("Album name")
        .activates_default(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let ui_for_choice = ui.clone();
    let entry_for_choice = entry.clone();
    dialog.connect_response(None, move |dlg, response| {
        if response != "create" {
            return;
        }
        let name = entry_for_choice.text().to_string();
        if name.trim().is_empty() {
            return;
        }
        let ui = ui_for_choice.clone();
        glib::MainContext::default().spawn_local(async move {
            match ui.ctx.api_client.create_album(name.trim()).await {
                Ok(_) => {
                    refresh_library_after_mutation(ui.clone(), false);
                }
                Err(err) => log::error!("Create album failed: {}", err),
            }
        });
        dlg.close();
    });
    dialog.present(Some(&ui.window));
}

/// Asynchronously fetch the latest albums from the server and redraw the album views.
///
/// Late-arriving fetches must not hijack the stack: if the user navigated to
/// another tab while we were waiting on the network, we still bind the data
/// (so the grid is current when they come back) but we do *not* force the
/// stack child to "albums" or "error".
fn refresh_albums_view(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_library_albums().await {
            Ok(albums) => {
                let on_click = album_click_handler(ui.clone());
                populate_albums(&ui.albums, ui.ctx.clone(), albums, on_click);
                if matches!(
                    ui.content_stack.visible_child_name().as_deref(),
                    Some("albums") | Some("loading")
                ) {
                    ui.content_stack.set_visible_child_name("albums");
                }
            }
            Err(err) => {
                log::warn!("Albums fetch failed: {}", err);
                if !ui.albums.populated.get()
                    && ui.content_stack.visible_child_name().as_deref() == Some("loading")
                {
                    ui.error_label
                        .set_label(&format!("Could not load albums: {}", err));
                    ui.content_stack.set_visible_child_name("error");
                }
            }
        }
    });
}

/// Produce a click callback handler for handling album activation events.
fn album_click_handler(ui: Rc<LibraryWindowUi>) -> AlbumClick {
    Rc::new(move |id: &str, name: String| {
        sidebar_dispatch(
            ui.clone(),
            LibrarySource::Album {
                id: id.to_string(),
                name,
            },
        );
    })
}

/// Apply header control layout adjustments when switching view modes.
fn apply_timeline_ui_state(ui: &LibraryWindowUi, source: &LibrarySource) {
    let timeline_allowed = matches!(source, LibrarySource::AllAssets | LibrarySource::Timeline);
    let timeline_active = matches!(source, LibrarySource::Timeline);
    ui.ctx
        .library_timeline_active
        .store(timeline_active, std::sync::atomic::Ordering::Relaxed);
    ui.timeline_toggle.set_sensitive(timeline_allowed);
    if ui.timeline_toggle.is_active() != timeline_active {
        ui.timeline_toggle.set_active(timeline_active);
    }
    ui.timeline_banner.set_visible(timeline_active);
    if timeline_active {
        ui.sort_mode.set_selected(0);
    }

    let is_local = matches!(
        source,
        LibrarySource::LocalAll
            | LibrarySource::LocalSearch { .. }
            | LibrarySource::AlbumLocal { .. }
    );
    let is_unified = matches!(
        source,
        LibrarySource::Unified
            | LibrarySource::UnifiedSearch { .. }
            | LibrarySource::AlbumUnified { .. }
    );
    let in_album = matches!(
        source,
        LibrarySource::Album { .. }
            | LibrarySource::AlbumLocal { .. }
            | LibrarySource::AlbumUnified { .. }
    );
    let remote_search_allowed = !is_local && !is_unified;
    let is_narrow = ui.narrow.get();
    ui.search_mode
        .set_visible(remote_search_allowed && !is_narrow);
    ui.filters_button
        .set_visible(remote_search_allowed && !is_narrow);
    if !remote_search_allowed {
        ui.search_mode.set_selected(0);
    }

    // Source-mode (Remote/Local/Unified) only relevant inside an album.
    ui.source_revealer.set_reveal_child(in_album);

    if in_album {
        ui.upload_button.set_tooltip_text(Some("Upload to album"));
    } else {
        ui.upload_button.set_tooltip_text(Some("Upload to library"));
    }

    // Keep source dropdown visually consistent with the active source so
    // sidebar selections don't leave it showing the wrong tab.
    let target = if is_local {
        1
    } else if is_unified {
        2
    } else {
        0
    };
    if ui.source_mode.selected() != target {
        ui.source_mode_suppressed.set(true);
        ui.source_mode.set_selected(target);
        ui.source_mode_suppressed.set(false);
    }

    refresh_album_link_row(ui, source);
}

/// Compute and refresh the month-year overlay text dynamically while scrubbing the timeline list.
fn update_timeline_banner_if_active(ui: &Rc<LibraryWindowUi>, adj: &gtk::Adjustment) {
    let state = ui.ctx.library_state.lock();
    if !matches!(state.source, LibrarySource::Timeline) {
        return;
    }
    if state.assets.is_empty() {
        ui.timeline_banner.set_label("");
        return;
    }

    let max = (adj.upper() - adj.page_size()).max(1.0);
    let frac = (adj.value() / max).clamp(0.0, 1.0);
    let idx = ((state.assets.len() as f64) * frac) as usize;
    let idx = idx.min(state.assets.len() - 1);
    let label = month_year_label(&state.assets[idx].created_at);
    ui.timeline_banner.set_label(&label);
}

/// Convert an ISO 8601 string to a human-readable "Month Year" label.
fn month_year_label(iso: &str) -> String {
    use chrono::{DateTime, Datelike};
    if let Ok(dt) = DateTime::parse_from_rfc3339(iso) {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        let m = dt.month0() as usize;
        if let Some(name) = MONTHS.get(m) {
            return format!("{} {}", name, dt.year());
        }
    }
    iso.chars().take(7).collect()
}

/// Retrieve albums from the API and update the side navigation sidebar entries.
fn load_albums(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            match ui.ctx.api_client.fetch_library_albums().await {
                Ok(albums) => {
                    ui.ctx.library_state.lock().load_albums(albums);
                    reload_sidebar(&ui);
                }
                Err(err) => {
                    // Sidebar-list refresh: never hijack the user's current
                    // tab with an error page just because the albums list
                    // couldn't reload. Only surface the error view when the
                    // user is actively waiting on something.
                    log::warn!("Albums sidebar refresh failed: {}", err);
                    if ui.content_stack.visible_child_name().as_deref() == Some("loading") {
                        ui.error_label
                            .set_label(&format!("Could not load albums: {}", err));
                        ui.content_stack.set_visible_child_name("error");
                    }
                }
            }
        }
    ));
}

/// Fetch and populate active server version details and stats into the UI.
fn load_status(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let stats = ui.ctx.api_client.fetch_server_stats().await.ok();
            let about = ui.ctx.api_client.fetch_server_about().await.ok();
            let route = ui.ctx.api_client.active_route_label().await;

            {
                let mut state = ui.ctx.library_state.lock();
                state.set_status(stats, about);
            }
            update_footer(&ui, route);
        }
    ));
}

/// Load smart/metadata sections onto the Explore dashboard view.
///
/// Switches to the Explore stack child immediately so the user sees the
/// section headers + per-section spinners while data is in flight. Each of
/// the three fetches runs as an independent task, so a slow endpoint never
/// blocks the others — first contentful paint happens as soon as the
/// fastest section returns.
fn load_explore_landing(ui: Rc<LibraryWindowUi>) {
    if ui.explore.populated.get() {
        ui.content_stack.set_visible_child_name("explore");
        return;
    }
    ui.explore.populated.set(true);
    let ctx = ui.ctx.clone();
    explore_view::wire_people_filter(&ui.explore, ctx.clone(), || {});
    explore_view::show_loading(&ui.explore);
    ui.content_stack.set_visible_child_name("explore");

    let mctx = glib::MainContext::default();

    mctx.spawn_local(clone!(
        #[strong]
        ui,
        #[strong]
        ctx,
        async move {
            let people = ctx.api_client.fetch_people(true).await.unwrap_or_default();
            let click_ui = ui.clone();
            explore_view::populate_people(&ui.explore, ctx.clone(), people, move |id, name| {
                let filters = MetadataSearchFilters {
                    person_ids: Some(vec![id]),
                    ..Default::default()
                };
                let request = click_ui.ctx.library_state.lock().switch_source(
                    LibrarySource::AdvancedSearch {
                        filters: Box::new(filters),
                    },
                );
                click_ui.search_entry.set_text(&name);
                apply_timeline_ui_state(&click_ui, &request.1);
                load_source_page(click_ui.clone(), request, false);
            });
        }
    ));

    mctx.spawn_local(clone!(
        #[strong]
        ui,
        #[strong]
        ctx,
        async move {
            let places = ctx.api_client.fetch_all_places().await.unwrap_or_default();
            let click_ui = ui.clone();
            explore_view::populate_places(&ui.explore, ctx.clone(), places, move |_kind, value| {
                let next = LibrarySource::AdvancedSearch {
                    filters: Box::new(MetadataSearchFilters {
                        city: Some(value.clone()),
                        ..Default::default()
                    }),
                };
                let request = click_ui.ctx.library_state.lock().switch_source(next);
                click_ui.search_entry.set_text(&value);
                apply_timeline_ui_state(&click_ui, &request.1);
                load_source_page(click_ui.clone(), request, false);
            });
        }
    ));

    mctx.spawn_local(clone!(
        #[strong]
        ui,
        #[strong]
        ctx,
        async move {
            let sections = ctx.api_client.fetch_explore().await.unwrap_or_default();
            let click_ui = ui.clone();
            explore_view::populate_explore(
                &ui.explore,
                ctx.clone(),
                sections,
                move |_kind, value| {
                    let next = LibrarySource::SmartSearch {
                        query: value.clone(),
                    };
                    let request = click_ui.ctx.library_state.lock().switch_source(next);
                    click_ui.search_entry.set_text(&value);
                    apply_timeline_ui_state(&click_ui, &request.1);
                    load_source_page(click_ui.clone(), request, false);
                },
            );
        }
    ));
}

/// Load a paginated subset of assets for the current source.
fn load_source_page(ui: Rc<LibraryWindowUi>, request: (u64, LibrarySource, u32), append: bool) {
    if matches!(request.1, LibrarySource::Explore) {
        load_explore_landing(ui);
        return;
    }
    if !append {
        ui.content_stack.set_visible_child_name("loading");
    }
    log::debug!("Loading library source {:?} page={}", request.1, request.2);
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let (generation, source, page) = request;
            let order = ui.ctx.library_state.lock().sort_mode.server_order();
            let result: Result<(Vec<LibraryAsset>, bool), String> = match source.clone() {
                LibrarySource::AllAssets | LibrarySource::Timeline => {
                    ui.ctx
                        .api_client
                        .search_metadata("", page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::Explore => unreachable!("intercepted above"),
                LibrarySource::Album { id, .. } => {
                    ui.ctx
                        .api_client
                        .fetch_album_assets(&id, page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::SmartSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_smart(&query, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::OcrSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_ocr(&query, page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::MetadataSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::AdvancedSearch { filters } => {
                    let mut filters = (*filters).clone();
                    filters.order = order;
                    ui.ctx
                        .api_client
                        .search_metadata_with_filters(&filters, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::LocalAll => {
                    // Local enumeration is bounded — single synthetic page.
                    if page > 1 {
                        Ok((Vec::new(), false))
                    } else {
                        let locals = enumerate_local(ui.ctx.clone()).await;
                        Ok((
                            locals.into_iter().map(local_to_library_asset).collect(),
                            false,
                        ))
                    }
                }
                LibrarySource::LocalSearch { query } => {
                    if page > 1 {
                        Ok((Vec::new(), false))
                    } else {
                        let locals = enumerate_local(ui.ctx.clone()).await;
                        let filtered = filter_by_filename(locals, &query);
                        Ok((
                            filtered.into_iter().map(local_to_library_asset).collect(),
                            false,
                        ))
                    }
                }
                LibrarySource::Unified => {
                    let remote = ui
                        .ctx
                        .api_client
                        .search_metadata("", page, PAGE_SIZE, order)
                        .await;
                    merge_unified_page(remote, page, &ui, None).await
                }
                LibrarySource::UnifiedSearch { query } => {
                    let remote = ui
                        .ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE, order)
                        .await;
                    merge_unified_page(remote, page, &ui, Some(&query)).await
                }
                LibrarySource::AlbumLocal { name, .. } => {
                    if page > 1 {
                        Ok((Vec::new(), false))
                    } else {
                        match linked_entry_path_for_album(&ui, &name) {
                            Some(path) => {
                                let locals = enumerate_local_for_entry(ui.ctx.clone(), path).await;
                                Ok((
                                    locals.into_iter().map(local_to_library_asset).collect(),
                                    false,
                                ))
                            }
                            None => Ok((Vec::new(), false)),
                        }
                    }
                }
                LibrarySource::AlbumUnified { id, name } => {
                    let remote = ui
                        .ctx
                        .api_client
                        .fetch_album_assets(&id, page, PAGE_SIZE, order)
                        .await;
                    merge_album_unified_page(remote, page, &ui, &name).await
                }
            };

            match result {
                Ok((items, has_more)) => {
                    {
                        let mut state = ui.ctx.library_state.lock();
                        let applied = if append {
                            state.append_assets_with_more(generation, items, has_more)
                        } else {
                            state.replace_assets_with_more(generation, items, has_more)
                        };
                        if !applied {
                            return;
                        }
                        if append {
                            ui.grid
                                .model
                                .extend(&ui.ctx, &state.assets, &state.sort_mode);
                        } else {
                            ui.grid
                                .model
                                .reset(&ui.ctx, &state.assets, &state.sort_mode);
                        }
                    }
                    // Lock is released before touching GTK widgets so that
                    // signal handlers triggered by the stack transition
                    // can safely re-acquire library_state.
                    sync_content_state(&ui);
                    reload_sidebar(&ui);
                    update_timeline_banner_if_active(&ui, &ui.grid.scrolled.vadjustment());
                }
                Err(err) => {
                    {
                        let mut state = ui.ctx.library_state.lock();
                        state.mark_error(generation, err.clone());
                    }
                    // Lock dropped before GTK calls (same pattern as Ok path).
                    ui.error_label
                        .set_label(&format!("Could not load library assets: {}", err));
                    ui.content_stack.set_visible_child_name("error");
                }
            }
        }
    ));
}

/// Repopulate and select the active album or fixed row in the side navigation sidebar.
fn reload_sidebar(ui: &Rc<LibraryWindowUi>) {
    while let Some(row) = ui.sidebar.albums_list.first_child() {
        ui.sidebar.albums_list.remove(&row);
    }

    let selected_source = ui.ctx.library_state.lock().source.clone();
    let albums = ui.ctx.library_state.lock().albums.clone();
    for album in albums {
        let subtitle = format!("{} asset(s)", album.asset_count);
        let action = libadwaita::ActionRow::builder()
            .title(&album.album_name)
            .subtitle(&subtitle)
            .title_lines(1)
            .subtitle_lines(1)
            .build();
        let row = gtk::ListBoxRow::builder()
            .tooltip_text(format!("{}:{}", album.id, album.album_name))
            .child(&action)
            .build();
        ui.sidebar.albums_list.append(&row);
    }

    match selected_source {
        LibrarySource::Timeline => {
            select_fixed_row(&ui.sidebar.fixed_list, "photos");
            ui.sidebar.albums_list.unselect_all();
        }
        LibrarySource::Explore => {
            select_fixed_row(&ui.sidebar.fixed_list, "explore");
            ui.sidebar.albums_list.unselect_all();
        }
        LibrarySource::Album { id, .. }
        | LibrarySource::AlbumLocal { id, .. }
        | LibrarySource::AlbumUnified { id, .. } => {
            ui.sidebar.fixed_list.unselect_all();
            ui.sidebar_suppressed.set(true);
            let mut child = ui.sidebar.albums_list.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
                    && row.tooltip_text().as_deref().is_some_and(|tooltip| {
                        tooltip.split_once(':').map(|(prefix, _)| prefix) == Some(id.as_str())
                    })
                {
                    ui.sidebar.albums_list.select_row(Some(&row));
                    ui.sidebar_suppressed.set(false);
                    break;
                }
                child = next;
            }
            ui.sidebar_suppressed.set(false);
        }
        _ => {
            ui.sidebar.fixed_list.unselect_all();
            ui.sidebar.albums_list.unselect_all();
        }
    }
}

/// Helper to select a sidebar row matching a specific string key.
fn select_fixed_row(list: &gtk::ListBox, key: &str) {
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
            && row.tooltip_text().as_deref() == Some(key)
        {
            list.select_row(Some(&row));
            return;
        }
        child = next;
    }
}

/// Sync the visibility of the grid, loading, empty, and error page widgets.
///
/// The lock on `library_state` is released **before** calling
/// `set_visible_child_name` because that GTK call triggers widget
/// realization, factory binds, and signal handlers that may need to
/// re-acquire the same lock.  Holding it across the call caused a
/// parking_lot deadlock on first library open.
fn sync_content_state(ui: &LibraryWindowUi) {
    let (child_name, error_msg) = {
        let state = ui.ctx.library_state.lock();
        match &state.load_state {
            LibraryLoadState::Idle | LibraryLoadState::Loading => ("loading", None),
            LibraryLoadState::Loaded => ("grid", None),
            LibraryLoadState::Empty => ("empty", None),
            LibraryLoadState::Error(msg) => ("error", Some(msg.clone())),
        }
    };
    if let Some(msg) = error_msg {
        ui.error_label.set_label(&msg);
    }
    ui.content_stack.set_visible_child_name(child_name);
}

/// Update the status sidebar rows with the current server route and statistics.
fn update_footer(ui: &LibraryWindowUi, route: Option<String>) {
    let (stats_text, about_text) = {
        let state = ui.ctx.library_state.lock();
        let stats = state
            .status
            .stats
            .as_ref()
            .map(|stats| format!("{} photos, {} videos", stats.images, stats.videos))
            .unwrap_or_else(|| "Statistics unavailable".to_string());
        let about = state
            .status
            .about
            .as_ref()
            .map(|about| format!("Immich {}", about.version))
            .unwrap_or_else(|| "Version unavailable".to_string());
        (stats, about)
    };
    let route_subtitle = route
        .as_deref()
        .map(|route| match route {
            "LAN" => "Connected through LAN",
            "WAN" => "Connected through WAN",
            _ => "Connected through configured server",
        })
        .unwrap_or("Offline");
    ui.sidebar.connection_row.set_subtitle(route_subtitle);
    ui.sidebar
        .server_row
        .set_subtitle(&format!("{stats_text} | {about_text}"));
}

/// Synchronize the ongoing upload/download progress indicator bar and rate information text.
fn update_transfer_ui(ui: &LibraryWindowUi) {
    let transfer = {
        let mut state = ui.ctx.state.lock();
        if state.transfer.active
            && state.transfer.active_uploads == 0
            && state.transfer.active_downloads == 0
        {
            // Guard against sessions that were opened but never queued.
            state.transfer.reset_runtime();
        }
        state.transfer.clone()
    };
    if !transfer.active {
        ui.transfer_bar.remove_css_class("active");
        ui.transfer_progress.set_fraction(0.0);
        ui.transfer_icon.set_visible(false);
        let idle_summary =
            if transfer.last_upload_avg_bps > 0.0 || transfer.last_download_avg_bps > 0.0 {
                format!(
                    "Idle  Last upload avg {}  Last download avg {}",
                    format_rate(transfer.last_upload_avg_bps),
                    format_rate(transfer.last_download_avg_bps)
                )
            } else {
                "Idle  No recent transfer session".to_string()
            };
        ui.transfer_label.set_label(&idle_summary);
        return;
    }
    ui.transfer_bar.add_css_class("active");

    let icon_name = match transfer.direction {
        TransferDirection::Upload => "mimick-upload-symbolic",
        TransferDirection::Download => "mimick-download-symbolic",
    };
    ui.transfer_icon.set_icon_name(Some(icon_name));
    ui.transfer_icon.set_visible(true);

    let detail = transfer
        .active_item_label
        .as_deref()
        .unwrap_or(match transfer.direction {
            TransferDirection::Upload => "queued asset",
            TransferDirection::Download => "selected asset",
        });
    let live_speed = format_rate(transfer.instant_bps);
    let avg_speed = format_rate(transfer.session_avg_bps);
    ui.transfer_label
        .set_label(&format!("{detail}  {live_speed}  avg {avg_speed}"));

    match transfer.total_bytes {
        Some(total) if total > 0 => {
            ui.transfer_progress.set_show_text(false);
            ui.transfer_progress
                .set_fraction((transfer.current_bytes as f64 / total as f64).clamp(0.0, 1.0));
        }
        _ => {
            ui.transfer_progress.pulse();
        }
    }
}

/// Construct a styled placeholder view containing an icon, header title, and description text.
/// Centered Mimick-icon spinner used while library data is fetching. Shares
/// the `mimick-loader-icon` animation with the lightbox image-load spinner.
fn build_loading_view() -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .vexpand(true)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .css_classes(vec!["mimick-empty".to_string()])
        .build();
    let icon = gtk::Image::builder()
        .icon_name("dev.nicx.mimick")
        .pixel_size(72)
        .css_classes(["mimick-loader-icon"])
        .build();
    let title = gtk::Label::builder()
        .label("Loading…")
        .css_classes(vec!["mimick-empty-title".to_string()])
        .build();
    let subtitle = gtk::Label::builder()
        .label("Fetching library data from the Immich server")
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .justify(gtk::Justification::Center)
        .max_width_chars(28)
        .css_classes(vec!["mimick-empty-subtitle".to_string()])
        .build();
    container.append(&icon);
    container.append(&title);
    container.append(&subtitle);
    container
}

fn build_status_view(icon_name: &str, title: &str, subtitle: &str) -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .vexpand(true)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .css_classes(vec!["mimick-empty".to_string()])
        .build();
    let icon = gtk::Image::builder()
        .icon_name(icon_name)
        .pixel_size(64)
        .build();
    icon.add_css_class("dim-label");
    let title_label = gtk::Label::builder()
        .label(title)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .justify(gtk::Justification::Center)
        .max_width_chars(28)
        .css_classes(vec!["mimick-empty-title".to_string()])
        .build();
    let subtitle_label = gtk::Label::builder()
        .label(subtitle)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .justify(gtk::Justification::Center)
        .max_width_chars(28)
        .css_classes(vec!["mimick-empty-subtitle".to_string()])
        .build();
    container.append(&icon);
    container.append(&title_label);
    container.append(&subtitle_label);
    container
}

/// Convert a Base64-encoded Immich checksum string to its standard hexadecimal representation.
pub(super) fn immich_checksum_to_hex(b64: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Build a synthetic `LibraryAsset` from a physical local folder file.
fn local_to_library_asset(local: LocalAsset) -> LibraryAsset {
    LibraryAsset {
        id: format!("{}{}", LOCAL_ID_PREFIX, local.path.display()),
        filename: local.filename,
        mime_type: local.mime,
        created_at: local.created_at,
        asset_type: local.asset_type.to_string(),
        thumbhash: None,
        width: None,
        height: None,
        checksum: None,
        exif_info: None,
    }
}

/// Merge a page of remote API assets with local files that haven't been synchronized yet.
async fn merge_unified_page(
    remote: Result<(Vec<LibraryAsset>, bool), String>,
    page: u32,
    ui: &Rc<LibraryWindowUi>,
    query: Option<&str>,
) -> Result<(Vec<LibraryAsset>, bool), String> {
    let (mut remote, has_more) = remote?;
    if page > 1 {
        return Ok((remote, has_more));
    }

    let mut locals = enumerate_local(ui.ctx.clone()).await;
    if let Some(q) = query {
        locals = filter_by_filename(locals, q);
    }

    let synced_paths: std::collections::HashSet<String> = {
        remote
            .iter()
            .filter_map(|a| a.checksum.as_deref())
            .filter_map(immich_checksum_to_hex)
            .filter_map(|hex| ui.ctx.sync_index.local_path_for_checksum(&hex))
            .collect()
    };

    let mut local_rows: Vec<LibraryAsset> = locals
        .into_iter()
        .filter(|l| !synced_paths.contains(&l.path.display().to_string()))
        .map(local_to_library_asset)
        .collect();

    local_rows.append(&mut remote);
    Ok((local_rows, has_more))
}

/// Return the path of the watch entry linked to `album_name`, if any.
/// Find the local directory watch path mapped to the specified album.
fn linked_entry_path_for_album(ui: &Rc<LibraryWindowUi>, album_name: &str) -> Option<String> {
    let entries = ui.ctx.live_watch_paths.lock().clone();
    crate::config::watch_entry_for_album(album_name, &entries).map(|e| e.path().to_string())
}

/// Album-scoped variant of `merge_unified_page`: takes the album's asset
/// page from the remote API and overlays sync state from the album's
/// linked local folder only — never from siblings.
/// Merge a page of remote album assets with local folder files linked to the album.
async fn merge_album_unified_page(
    remote: Result<(Vec<LibraryAsset>, bool), String>,
    page: u32,
    ui: &Rc<LibraryWindowUi>,
    album_name: &str,
) -> Result<(Vec<LibraryAsset>, bool), String> {
    let (mut remote, has_more) = remote?;
    if page > 1 {
        return Ok((remote, has_more));
    }

    let locals = match linked_entry_path_for_album(ui, album_name) {
        Some(path) => enumerate_local_for_entry(ui.ctx.clone(), path).await,
        None => Vec::new(),
    };

    let synced_paths: std::collections::HashSet<String> = {
        remote
            .iter()
            .filter_map(|a| a.checksum.as_deref())
            .filter_map(immich_checksum_to_hex)
            .filter_map(|hex| ui.ctx.sync_index.local_path_for_checksum(&hex))
            .collect()
    };

    let mut local_rows: Vec<LibraryAsset> = locals
        .into_iter()
        .filter(|l| !synced_paths.contains(&l.path.display().to_string()))
        .map(local_to_library_asset)
        .collect();

    local_rows.append(&mut remote);
    Ok((local_rows, has_more))
}

#[cfg(test)]
mod texture_decoder_tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn assert_fixture_texture(name: &str, dimensions: (i32, i32)) {
        let texture = load_texture_blocking(&fixture(name))
            .unwrap_or_else(|| panic!("fixture `{name}` should decode into a texture"));
        assert_eq!(
            (texture.width(), texture.height()),
            dimensions,
            "fixture `{name}` decoded with unexpected dimensions"
        );
    }

    #[test]
    fn routes_special_lightbox_formats_before_loader_fallbacks() {
        for ext in crate::media_kinds::RAW_EXTENSIONS.iter() {
            assert_eq!(
                texture_decoder_for_path(std::path::Path::new(&format!("camera.{ext}"))),
                TextureDecoder::Raw,
                ".{ext} should run through the RAW pipeline"
            );
        }

        for ext in ["avif", "heic", "heif", "hif"] {
            assert_eq!(
                texture_decoder_for_path(std::path::Path::new(&format!("phone.{ext}"))),
                TextureDecoder::Heif
            );
        }
        assert_eq!(
            texture_decoder_for_path(std::path::Path::new("wide.JXL")),
            TextureDecoder::JpegXl
        );
    }

    #[test]
    fn every_supported_image_extension_has_a_decoder_route() {
        for ext in crate::media_kinds::SUPPORTED.iter() {
            let Some(mime) = crate::media_kinds::mime_for(ext) else {
                continue;
            };
            if crate::media_kinds::asset_kind(mime) == crate::media_kinds::AssetKind::Image {
                let route =
                    texture_decoder_for_path(std::path::Path::new(&format!("fixture.{ext}")));
                assert!(
                    matches!(
                        route,
                        TextureDecoder::Raw
                            | TextureDecoder::Heif
                            | TextureDecoder::JpegXl
                            | TextureDecoder::Svg
                            | TextureDecoder::Jpeg
                            | TextureDecoder::Webp
                            | TextureDecoder::Jpeg2k
                            | TextureDecoder::Psd
                            | TextureDecoder::Pixbuf
                            | TextureDecoder::ImageFallback
                    ),
                    "image extension `.{ext}` has no lightbox decoder route"
                );
            }
        }
    }

    #[test]
    fn fixture_standard_formats_decode_to_textures() {
        for name in ["sample.jpg", "sample.webp"] {
            assert_fixture_texture(name, (16, 12));
        }
    }

    #[test]
    fn fixture_svg_decodes_to_texture() {
        assert_fixture_texture("sample.svg", (16, 12));
    }

    #[test]
    fn fixture_heif_family_decodes_to_textures() {
        assert_fixture_texture("sample.avif", (16, 12));
        assert_fixture_texture("sample.heic", (64, 64));
    }

    #[test]
    fn fixture_jpegxl_decodes_to_texture() {
        assert_fixture_texture("sample.jxl", (16, 12));
    }

    #[test]
    fn fixture_dng_decodes_to_texture() {
        // The synthetic 16x12 fixture has no embedded preview; force full decode.
        RAW_FULL_DECODE.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_fixture_texture("sample.dng", (16, 12));
        RAW_FULL_DECODE.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn largest_embedded_jpeg_picks_biggest_soi_payload() {
        // Build a synthetic buffer with two JPEG SOI/EOI blocks: a small one
        // (below the 4KB minimum — should be skipped) and a large one.
        // Both use valid JPEG marker structure with declared segment lengths.

        /// Build a minimal valid JPEG with an APP marker segment of the given
        /// filler size, terminated by EOI.
        fn build_jpeg(app_marker: u8, filler_size: usize) -> Vec<u8> {
            // APP segment: FF <marker> <len_hi> <len_lo> <filler...>
            let seg_len = (filler_size + 2) as u16; // +2 for the length field itself
            let mut v = vec![0xFFu8, 0xD8, 0xFF, app_marker];
            v.extend_from_slice(&seg_len.to_be_bytes());
            v.extend_from_slice(&vec![0x42u8; filler_size]);
            v.extend_from_slice(&[0xFF, 0xD9]); // EOI
            v
        }

        let small = build_jpeg(0xE0, 30); // ~36 bytes — well below MIN_EMBEDDED_JPEG_SIZE
        let large = build_jpeg(0xE1, 8192); // ~8200 bytes — above threshold

        let mut buf = Vec::new();
        buf.extend_from_slice(&[0x00; 16]);
        buf.extend_from_slice(&small);
        buf.extend_from_slice(&[0x00; 64]);
        buf.extend_from_slice(&large);
        buf.extend_from_slice(&[0x00; 16]);

        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&buf).unwrap();
        let got = extract_largest_embedded_jpeg(tmp.path()).expect("scanner should find a JPEG");

        assert_eq!(got.len(), large.len(), "should return the larger payload");
        assert_eq!(&got[..4], &[0xFF, 0xD8, 0xFF, 0xE1]);
        assert_eq!(&got[got.len() - 2..], &[0xFF, 0xD9]);
    }

    #[test]
    fn largest_embedded_jpeg_skips_byte_stuffed_ff00() {
        // FF 00 inside entropy data must NOT be mistaken for an SOI (which is
        // FF D8 FF xx with xx != 00). A file with only an FF 00 sequence and
        // no real SOI should return None.
        let buf: Vec<u8> = vec![0x00, 0xFF, 0x00, 0xFF, 0xD8, 0xFF, 0x00, 0x42];
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&buf).unwrap();
        let got = extract_largest_embedded_jpeg(tmp.path());
        assert!(got.is_none(), "byte-stuffed FF 00 must not match as SOI");
    }

    #[test]
    fn largest_embedded_jpeg_accepts_implicit_eoi_at_eof() {
        // Some DNG/CR3/RW2 files omit the trailing FF D9 — the scanner should
        // accept the payload up to EOF when no explicit EOI is found.
        let seg_len = (8192 + 2) as u16;
        let mut jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xE0];
        jpeg.extend_from_slice(&seg_len.to_be_bytes());
        jpeg.extend_from_slice(&vec![0x42u8; 8192]);
        // NO trailing FF D9 — implicit end at EOF.

        let mut buf = Vec::new();
        buf.extend_from_slice(&[0x00; 16]);
        buf.extend_from_slice(&jpeg);

        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&buf).unwrap();
        let got =
            extract_largest_embedded_jpeg(tmp.path()).expect("scanner should accept implicit EOI");

        // The payload should span from the SOI to the end of the file.
        assert_eq!(got.len(), buf.len() - 16, "should capture SOI to EOF");
        assert_eq!(&got[..4], &[0xFF, 0xD8, 0xFF, 0xE0]);
    }

    #[test]
    fn jpeg_exif_orientation_parses_tag_0112() {
        // Hand-built minimal JPEG: SOI + APP1(Exif/TIFF little-endian, IFD0
        // with a single Orientation=6 entry) + EOI. Verifies our parser walks
        // segments, finds tag 0x0112, and returns the 16-bit SHORT value.
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"II"); // little-endian
        tiff.extend_from_slice(&0x002A_u16.to_le_bytes());
        tiff.extend_from_slice(&8_u32.to_le_bytes()); // IFD0 offset
        tiff.extend_from_slice(&1_u16.to_le_bytes()); // one entry
        tiff.extend_from_slice(&0x0112_u16.to_le_bytes()); // tag = Orientation
        tiff.extend_from_slice(&3_u16.to_le_bytes()); // type = SHORT
        tiff.extend_from_slice(&1_u32.to_le_bytes()); // count
        tiff.extend_from_slice(&6_u16.to_le_bytes()); // value (low half)
        tiff.extend_from_slice(&0_u16.to_le_bytes()); // value (high half)

        let mut app1: Vec<u8> = Vec::new();
        app1.extend_from_slice(b"Exif\0\0");
        app1.extend_from_slice(&tiff);
        let app1_len = (app1.len() + 2) as u16;

        let mut jpeg: Vec<u8> = vec![0xFF, 0xD8]; // SOI
        jpeg.push(0xFF);
        jpeg.push(0xE1); // APP1
        jpeg.extend_from_slice(&app1_len.to_be_bytes());
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

        assert_eq!(read_jpeg_exif_orientation(&jpeg), Some(6));
    }

    #[test]
    fn jpeg_exif_orientation_none_when_missing() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xD9];
        assert_eq!(read_jpeg_exif_orientation(&jpeg), None);
    }

    #[test]
    fn is_lossless_jpeg_detects_sof3() {
        // SOI + SOF3 marker (FF C3) with a 10-byte segment + EOI.
        let mut lossless = vec![0xFFu8, 0xD8, 0xFF, 0xC3];
        lossless.extend_from_slice(&12_u16.to_be_bytes()); // segment length
        lossless.extend_from_slice(&[0x00; 10]); // segment body
        lossless.extend_from_slice(&[0xFF, 0xD9]); // EOI
        assert!(
            is_lossless_jpeg(&lossless, 0, lossless.len()),
            "SOF3 should be detected as lossless"
        );
    }

    #[test]
    fn is_lossless_jpeg_allows_baseline() {
        // SOI + SOF0 (baseline, FF C0) -- not lossless.
        let mut baseline = vec![0xFFu8, 0xD8, 0xFF, 0xC0];
        baseline.extend_from_slice(&12_u16.to_be_bytes());
        baseline.extend_from_slice(&[0x00; 10]);
        baseline.extend_from_slice(&[0xFF, 0xD9]);
        assert!(
            !is_lossless_jpeg(&baseline, 0, baseline.len()),
            "SOF0 should NOT be detected as lossless"
        );
    }

    #[test]
    fn is_lossless_jpeg_with_app_segments_before_sof() {
        // SOI + APP1 (FF E1) + SOF3 (FF C3) -- lossless despite APP prefix.
        let mut data = vec![0xFFu8, 0xD8];
        // APP1 segment with 20 bytes of filler
        data.extend_from_slice(&[0xFF, 0xE1]);
        data.extend_from_slice(&22_u16.to_be_bytes());
        data.extend_from_slice(&[0x42; 20]);
        // SOF3
        data.extend_from_slice(&[0xFF, 0xC3]);
        data.extend_from_slice(&12_u16.to_be_bytes());
        data.extend_from_slice(&[0x00; 10]);
        data.extend_from_slice(&[0xFF, 0xD9]);
        assert!(is_lossless_jpeg(&data, 0, data.len()));
    }

    #[test]
    fn soi_scanner_skips_lossless_jpeg() {
        // Build a file with TWO JPEGs: a large SOF3 (lossless, should be skipped)
        // and a smaller SOF0 (baseline, should be selected).

        fn build_jpeg_with_sof(sof_marker: u8, filler: usize) -> Vec<u8> {
            let mut v = vec![0xFFu8, 0xD8]; // SOI
            // SOF segment
            v.extend_from_slice(&[0xFF, sof_marker]);
            let seg_len = (filler + 2) as u16;
            v.extend_from_slice(&seg_len.to_be_bytes());
            v.extend_from_slice(&vec![0x42u8; filler]);
            v.extend_from_slice(&[0xFF, 0xD9]); // EOI
            v
        }

        let lossless = build_jpeg_with_sof(0xC3, 16000); // 16KB lossless -- larger
        let baseline = build_jpeg_with_sof(0xC0, 8000); // 8KB baseline -- smaller

        let mut buf = Vec::new();
        buf.extend_from_slice(&[0x00; 16]);
        buf.extend_from_slice(&lossless);
        buf.extend_from_slice(&[0x00; 32]);
        buf.extend_from_slice(&baseline);
        buf.extend_from_slice(&[0x00; 16]);

        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&buf).unwrap();
        let got = extract_largest_embedded_jpeg(tmp.path())
            .expect("scanner should find the baseline JPEG");

        // Should get the baseline (SOF0), not the larger lossless (SOF3).
        assert_eq!(got.len(), baseline.len());
        assert_eq!(&got[..2], &[0xFF, 0xD8]);
        assert_eq!(got[3], 0xC0, "should select the SOF0 JPEG, not SOF3");
    }

    #[test]
    fn thumbnail_prerotated_detects_portrait_on_landscape_sensor() {
        // Landscape sensor (6000x4000), flip=5 (90-degree), portrait thumb (480x640).
        assert!(is_thumbnail_prerotated(480, 640, 6000, 4000, 5));
    }

    #[test]
    fn thumbnail_prerotated_false_for_matching_aspect() {
        // Landscape sensor, flip=5, landscape thumb -- not pre-rotated.
        assert!(!is_thumbnail_prerotated(640, 480, 6000, 4000, 5));
    }

    #[test]
    fn thumbnail_prerotated_false_for_no_rotation() {
        // flip=0 (no rotation) -- never considered pre-rotated.
        assert!(!is_thumbnail_prerotated(480, 640, 6000, 4000, 0));
    }

    #[test]
    fn thumbnail_prerotated_false_for_180_rotation() {
        // flip=3 (180-degree) -- aspect doesn't change, never pre-rotated.
        assert!(!is_thumbnail_prerotated(480, 640, 6000, 4000, 3));
    }
}
