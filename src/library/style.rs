//! Library view CSS: subtle pulsing placeholder for loading thumbnails,
//! a muted error tile, and a colored connection dot for the status footer.
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
    border-radius: 8px;
    animation: mimick-pulse 1.4s ease-in-out infinite;
}

picture.mimick-thumbnail-loaded {
    border-radius: 8px;
}

picture.mimick-thumbnail-error {
    background-color: alpha(@error_color, 0.18);
    border-radius: 8px;
}

box.mimick-cell {
    border-radius: 10px;
    transition: background-color 150ms ease;
}

box.mimick-cell:hover {
    background-color: alpha(@view_fg_color, 0.05);
}

label.mimick-cell-name {
    font-size: 0.85em;
}

label.mimick-status-dot {
    font-size: 1em;
    margin-right: 4px;
}

label.mimick-status-dot.connected { color: @success_color; }
label.mimick-status-dot.offline   { color: @error_color; }

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
    opacity: 0.85;
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
