//! Central registry of supported media extensions, MIME types, and asset kinds.
//!
//! All file-extension lookups in the upload, watch, and library-view paths
//! resolve through this module so the three previously hand-maintained tables
//! cannot drift apart.

use phf::{phf_map, phf_set};
use std::path::Path;

/// Discriminant representing whether a media file is an image or a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// Asset is a static or animated image.
    Image,
    /// Asset is a video or motion stream.
    Video,
}

/// Static set of all supported file extensions for quick membership lookups.
/// Mirrors Immich's server-accepted formats; MPO is intentionally excluded.
pub static SUPPORTED: phf::Set<&'static str> = phf_set! {
    "3fr", "3gp", "3gpp", "ari", "arw", "avi", "avif", "bmp", "cap", "cin",
    "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "flv", "gif", "heic",
    "heif", "hif", "iiq", "insp", "insv", "jp2", "jpe", "jpeg", "jpg", "jxl",
    "k25", "kdc", "m2t", "m2ts", "m4v", "mkv", "mov", "mp4", "mpe", "mpeg",
    "mpg", "mrw", "mts", "mxf", "nef", "nrw", "orf", "ori", "pef",
    "png", "psd", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw", "svg",
    "tif", "tiff", "ts", "vob", "webm", "webp", "wmv", "x3f",
};

/// Camera RAW extensions -- subset of SUPPORTED that needs RAW-specific
/// decoding (libraw) instead of pixbuf or image-rs.
pub static RAW_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "3fr", "ari", "arw", "cap", "cin", "cr2", "cr3", "crw", "dcr", "dng",
    "erf", "fff", "iiq", "k25", "kdc", "mrw", "nef", "nrw", "orf", "ori",
    "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
};

/// Compile-time mapping from lowercased file extensions to standard MIME types.
static MIME_BY_EXT: phf::Map<&'static str, &'static str> = phf_map! {
    "avif" => "image/avif",
    "bmp" => "image/bmp",
    "gif" => "image/gif",
    "heic" => "image/heic",
    "heif" => "image/heif",
    "hif" => "image/heif",
    "insp" => "image/jpeg",
    "jpe" => "image/jpeg",
    "jpeg" => "image/jpeg",
    "jpg" => "image/jpeg",
    "jp2" => "image/jp2",
    "jxl" => "image/jxl",
    "png" => "image/png",
    "psd" => "image/vnd.adobe.photoshop",
    "svg" => "image/svg+xml",
    "tif" => "image/tiff",
    "tiff" => "image/tiff",
    "webp" => "image/webp",
    "3fr" => "image/x-hasselblad-3fr",
    "ari" => "image/x-arriflex-ari",
    "arw" => "image/x-sony-arw",
    "cap" => "image/x-phaseone-cap",
    "cin" => "image/cineon",
    "cr2" => "image/x-canon-cr2",
    "cr3" => "image/x-canon-cr3",
    "crw" => "image/x-canon-crw",
    "dcr" => "image/x-kodak-dcr",
    "dng" => "image/x-adobe-dng",
    "erf" => "image/x-epson-erf",
    "fff" => "image/x-hasselblad-fff",
    "iiq" => "image/x-phaseone-iiq",
    "k25" => "image/x-kodak-k25",
    "kdc" => "image/x-kodak-kdc",
    "mrw" => "image/x-minolta-mrw",
    "nef" => "image/x-nikon-nef",
    "nrw" => "image/x-nikon-nrw",
    "orf" => "image/x-olympus-orf",
    "ori" => "image/x-olympus-orf",
    "pef" => "image/x-pentax-pef",
    "raf" => "image/x-fuji-raf",
    "raw" => "image/x-panasonic-raw",
    "rw2" => "image/x-panasonic-rw2",
    "rwl" => "image/x-leica-rwl",
    "sr2" => "image/x-sony-sr2",
    "srf" => "image/x-sony-sr2",
    "srw" => "image/x-samsung-srw",
    "x3f" => "image/x-sigma-x3f",
    "3gp" => "video/3gpp",
    "3gpp" => "video/3gpp",
    "avi" => "video/x-msvideo",
    "flv" => "video/x-flv",
    "insv" => "video/mp4",
    "mp4" => "video/mp4",
    "m2t" => "video/mp2t",
    "m2ts" => "video/mp2t",
    "mts" => "video/mp2t",
    "ts" => "video/mp2t",
    "m4v" => "video/x-m4v",
    "mkv" => "video/x-matroska",
    "mpe" => "video/mpeg",
    "mpeg" => "video/mpeg",
    "mpg" => "video/mpeg",
    "mov" => "video/quicktime",
    "mxf" => "application/mxf",
    "vob" => "video/dvd",
    "webm" => "video/webm",
    "wmv" => "video/x-ms-wmv",
};

/// Lowercase the path's extension and return whether it is an accepted media file.
pub fn is_supported_path(path: &Path) -> bool {
    if path.is_dir() {
        return false;
    }
    extension_lower(path)
        .map(|ext| SUPPORTED.contains(ext.as_str()))
        .unwrap_or(false)
}

/// Check if a lowercased extension is present in the supported set.
pub fn is_supported_ext(ext: &str) -> bool {
    SUPPORTED.contains(ext)
}

/// Whether the extension is a camera RAW format.
pub fn is_raw_ext(ext: &str) -> bool {
    RAW_EXTENSIONS.contains(ext)
}

/// Whether the path's extension is a camera RAW format.
pub fn is_raw_path(path: &Path) -> bool {
    extension_lower(path)
        .map(|ext| RAW_EXTENSIONS.contains(ext.as_str()))
        .unwrap_or(false)
}

/// MIME type for a known extension (already lowercased).
pub fn mime_for(ext: &str) -> Option<&'static str> {
    MIME_BY_EXT.get(ext).copied()
}

/// MIME type for any path; falls back to `application/octet-stream` for unknowns.
pub fn mime_for_path(path: &Path) -> &'static str {
    extension_lower(path)
        .and_then(|ext| mime_for(&ext))
        .unwrap_or("application/octet-stream")
}

/// Determine the media kind based on the prefix of the MIME type.
pub fn asset_kind(mime: &str) -> AssetKind {
    if mime.starts_with("video/") {
        AssetKind::Video
    } else {
        AssetKind::Image
    }
}

/// Extract the extension from a path and return it lowercased.
fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_extension_has_a_mime() {
        for ext in SUPPORTED.iter() {
            assert!(
                MIME_BY_EXT.contains_key(ext),
                "extension `.{}` is supported but has no MIME mapping",
                ext
            );
        }
    }

    #[test]
    fn mime_for_path_handles_uppercase_and_unknowns() {
        assert_eq!(mime_for_path(Path::new("photo.PNG")), "image/png");
        assert_eq!(mime_for_path(Path::new("photo.jpe")), "image/jpeg");
        assert_eq!(
            mime_for_path(Path::new("doc.unknown")),
            "application/octet-stream"
        );
        assert_eq!(
            mime_for_path(Path::new("noext")),
            "application/octet-stream"
        );
    }

    #[test]
    fn asset_kind_buckets_video_prefix() {
        assert_eq!(asset_kind("video/mp4"), AssetKind::Video);
        assert_eq!(asset_kind("image/jpeg"), AssetKind::Image);
    }

    #[test]
    fn is_raw_path_detects_raw_extensions() {
        assert!(is_raw_path(Path::new("photo.NEF")));
        assert!(is_raw_path(Path::new("photo.cr3")));
        assert!(is_raw_path(Path::new("photo.DNG")));
        assert!(is_raw_path(Path::new("/some/path/IMG.ARW")));
        assert!(!is_raw_path(Path::new("photo.jpg")));
        assert!(!is_raw_path(Path::new("video.mp4")));
        assert!(!is_raw_path(Path::new("noext")));
    }
}
