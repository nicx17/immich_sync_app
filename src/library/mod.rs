//! Library view module -- browse, search, and download assets from an Immich server.
//!
//! This module is conditionally activated when `library_view_enabled` is true in config.
//! It provides the primary application window with album sidebar, thumbnail grid,
//! lightbox viewer, and search functionality.

pub mod asset_object;
pub mod grid_view;
pub mod sidebar;
pub mod thumbnail_cache;
