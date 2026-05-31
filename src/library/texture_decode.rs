//! Image decoding pipeline -- codec dispatch, RAW/HEIF/JXL/JPEG2K/SVG/PSD/WebP decoders,
//! embedded-preview extraction, EXIF orientation, and decode-cache management.
//!
//! This module was extracted from `library/mod.rs` to keep the main module
//! focused on window construction and navigation.

use gtk::prelude::*;

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
