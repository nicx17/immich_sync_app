//! Provides user-friendly display names for watch paths.
//!
//! Flatpak document-portal paths (`/run/user/.../doc/...`) are mapped
//! back to their original human-readable folder names. Regular paths
//! are shortened to their final component for compact UI display.

use std::path::Path;

/// Converts a stored watch path into a user-friendly label for display in the UI and logs.
pub fn display_watch_path(path: &str) -> String {
    if is_document_portal_path(path) {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string())
            .unwrap_or_else(|| "Selected Folder".to_string())
    } else {
        path.to_string()
    }
}

/// Returns an explanatory subtitle for special watch paths, if applicable.
pub fn watch_path_subtitle(_path: &str) -> Option<&'static str> {
    None
}

/// Returns the full path for regular folders, or the folder name for Flatpak portal paths.
/// Useful for contexts where the path is displayed inline (e.g., diagnostics, album linked folder) without a separate subtitle.
pub fn display_watch_path_inline(path: &str) -> String {
    display_watch_path(path)
}

/// Detects document-portal paths returned by the Flatpak file chooser portal.
pub fn is_document_portal_path(path: &str) -> bool {
    path.starts_with("/run/user/") && path.contains("/doc/")
}

#[cfg(test)]
mod tests {
    use super::{display_watch_path, is_document_portal_path};

    #[test]
    fn test_display_watch_path_for_portal_folder() {
        assert_eq!(
            display_watch_path("/run/user/1000/doc/abcd1234/Screenshots"),
            "Screenshots"
        );
    }

    #[test]
    fn test_display_watch_path_for_regular_folder() {
        assert_eq!(
            display_watch_path("/home/user/Pictures"),
            "/home/user/Pictures"
        );
        assert!(!is_document_portal_path("/home/user/Pictures"));
    }
}
