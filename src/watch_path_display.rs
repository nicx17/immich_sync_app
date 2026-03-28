//! Provides utilities for displaying user-friendly names for watch paths, especially those selected via Flatpak portals.

use std::path::Path;

const PORTAL_FOLDER_LABEL: &str = "Selected via Flatpak portal";

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
pub fn watch_path_subtitle(path: &str) -> Option<&'static str> {
    if is_document_portal_path(path) {
        Some(PORTAL_FOLDER_LABEL)
    } else {
        None
    }
}

/// Detects document-portal paths returned by the Flatpak file chooser portal.
pub fn is_document_portal_path(path: &str) -> bool {
    path.starts_with("/run/user/") && path.contains("/doc/")
}

#[cfg(test)]
mod tests {
    use super::{display_watch_path, is_document_portal_path, watch_path_subtitle};

    #[test]
    fn test_display_watch_path_for_portal_folder() {
        assert_eq!(
            display_watch_path("/run/user/1000/doc/abcd1234/Screenshots"),
            "Screenshots"
        );
    }

    #[test]
    fn test_watch_path_subtitle_for_portal_folder() {
        assert_eq!(
            watch_path_subtitle("/run/user/1000/doc/abcd1234/Screenshots"),
            Some("Selected via Flatpak portal")
        );
    }

    #[test]
    fn test_display_watch_path_for_regular_folder() {
        assert_eq!(
            display_watch_path("/home/nick/Pictures"),
            "/home/nick/Pictures"
        );
        assert!(!is_document_portal_path("/home/nick/Pictures"));
    }
}
