//! Library view CSS: subtle pulsing placeholder for loading thumbnails,
//! a muted error tile, and a rounded transfer progress treatment.
//!
//! Registered exactly once per process via `std::sync::OnceLock`.

use std::sync::OnceLock;

use gtk::CssProvider;
use gtk::gdk;
use gtk::style_context_add_provider_for_display;

const LIBRARY_CSS: &str = r#"
@keyframes mimick-pulse {
    0%   { background-color: alpha(@view_fg_color, 0.06); }
    50%  { background-color: alpha(@view_fg_color, 0.14); }
    100% { background-color: alpha(@view_fg_color, 0.06); }
}

picture.mimick-thumbnail-loading {
    background-color: alpha(@view_fg_color, 0.08);
    animation: mimick-pulse 1.4s ease-in-out infinite;
}

picture.mimick-thumbnail-loaded {
}

picture.mimick-thumbnail-error {
    background-color: alpha(@error_color, 0.18);
}

overlay.mimick-cell {
}

box.mimick-empty {
    padding: 32px;
}

label.mimick-empty-title {
    font-size: 1.2em;
    font-weight: bold;
}

label.mimick-empty-subtitle {
    opacity: 0.65;
}

label.mimick-timeline-banner {
    font-size: 1.05em;
    font-weight: 600;
    padding: 4px 8px;
    background-color: alpha(@accent_bg_color, 0.10);
    border-bottom: 1px solid alpha(@view_fg_color, 0.10);
}

image.mimick-status-badge {
    opacity: 0.90;
    color: white;
    -gtk-icon-shadow: 0 1px 3px rgba(0, 0, 0, 0.8);
}

image.mimick-video-badge {
    opacity: 0.92;
    color: white;
    -gtk-icon-shadow: 0 2px 4px rgba(0, 0, 0, 0.8);
}

picture.mimick-person-avatar {
    border-radius: 9999px;
    background-color: alpha(@view_fg_color, 0.08);
}

picture.mimick-explore-tile {
    border-radius: 6px;
    background-color: alpha(@view_fg_color, 0.08);
}

picture.mimick-thumbnail-square,
picture.mimick-thumbnail-square.mimick-thumbnail-loading,
picture.mimick-thumbnail-square.mimick-thumbnail-loaded,
picture.mimick-thumbnail-square.mimick-thumbnail-error {
    border-radius: 0;
}

box.mimick-transfer-shell {
    min-height: 36px;
    border-top: 1px solid alpha(@window_fg_color, 0.08);
    background: alpha(@window_fg_color, 0.03);
}

box.mimick-transfer-shell.active {
    background: alpha(@accent_bg_color, 0.04);
}

progressbar.mimick-transfer-progress {
    min-width: 180px;
    min-height: 18px;
}

progressbar.mimick-transfer-progress trough {
    min-height: 18px;
    border-radius: 999px;
    background: alpha(@window_fg_color, 0.12);
}

progressbar.mimick-transfer-progress progress {
    min-height: 18px;
    border-radius: 999px;
    background: mix(@accent_bg_color, #d16c96, 0.45);
}
"#;

static REGISTERED: OnceLock<()> = OnceLock::new();

/// Install the library-view stylesheet on the default display. Idempotent.
pub fn ensure_registered() {
    REGISTERED.get_or_init(|| {
        let Some(display) = gdk::Display::default() else {
            log::warn!("No default GDK display; library CSS not registered");
            return;
        };

        let provider = CssProvider::new();
        provider.load_from_string(LIBRARY_CSS);
        style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}
