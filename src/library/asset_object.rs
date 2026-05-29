//! GObject subclass representing a single asset in the library grid.
//!
//! Uses the `glib::Properties` derive macro so every field is a proper GObject property,
//! enabling `gtk::SortListModel`, expression bindings, and signal-based reactivity
//! without a future migration from `BoxedAnyObject`.

use glib::prelude::*;
use glib::subclass::prelude::*;
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;
    use glib::Properties;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::AssetObject)]
    pub struct AssetObject {
        /// Immich asset UUID.
        #[property(get, set)]
        id: RefCell<String>,

        /// Original filename on the server.
        #[property(get, set)]
        filename: RefCell<String>,

        /// MIME type (e.g. "image/jpeg").
        #[property(get, set)]
        mime_type: RefCell<String>,

        /// ISO 8601 creation timestamp from file metadata.
        #[property(get, set)]
        created_at: RefCell<String>,

        /// "IMAGE" or "VIDEO".
        #[property(get, set)]
        asset_type: RefCell<String>,

        /// Sync state indicator: 0 = remote only, 1 = local only, 2 = both.
        #[property(get, set)]
        sync_state: Cell<u32>,

        /// Optional thumbhash for placeholder rendering (base64-encoded).
        #[property(get, set)]
        thumbhash: RefCell<Option<String>>,

        /// Absolute path to the local copy on disk (LocalAsset, or matched
        /// remote with a local sibling); empty string when none.
        #[property(get, set)]
        local_path: RefCell<String>,

        /// Immich asset id when this row corresponds to a remote asset (or
        /// has been matched to one via checksum); empty string otherwise.
        #[property(get, set)]
        remote_id: RefCell<String>,

        /// Native pixel width of the asset; 0 = unknown (settled later from
        /// thumbnail decode for sources without EXIF metadata).
        #[property(get, set)]
        width: Cell<u32>,

        /// Native pixel height of the asset; 0 = unknown.
        #[property(get, set)]
        height: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AssetObject {
        const NAME: &'static str = "MimickAssetObject";
        type Type = super::AssetObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AssetObject {}
}

glib::wrapper! {
    /// A single library asset exposed as a full GObject for use in `gio::ListStore`.
    pub struct AssetObject(ObjectSubclass<imp::AssetObject>);
}

/// Constructor parameters for a remote `AssetObject`.
/// Pass `width` / `height` as 0 when unknown.
pub struct AssetInit<'a> {
    pub id: &'a str,
    pub filename: &'a str,
    pub mime_type: &'a str,
    pub created_at: &'a str,
    pub asset_type: &'a str,
    pub sync_state: u32,
    pub thumbhash: Option<&'a str>,
    pub width: u32,
    pub height: u32,
}

impl AssetObject {
    pub fn new(init: AssetInit<'_>) -> Self {
        glib::Object::builder()
            .property("id", init.id)
            .property("filename", init.filename)
            .property("mime-type", init.mime_type)
            .property("created-at", init.created_at)
            .property("asset-type", init.asset_type)
            .property("sync-state", init.sync_state)
            .property("thumbhash", init.thumbhash)
            .property("local-path", "")
            .property("remote-id", init.id)
            .property("width", init.width)
            .property("height", init.height)
            .build()
    }

    /// Build an AssetObject for a purely local asset (LocalOnly state).
    /// `id` is a synthetic row identity (use the absolute path) so the
    /// existing `id` plumbing in the grid factory keeps working.
    pub fn new_local(
        id: &str,
        filename: &str,
        mime_type: &str,
        created_at: &str,
        asset_type: &str,
        local_path: &str,
    ) -> Self {
        glib::Object::builder()
            .property("id", id)
            .property("filename", filename)
            .property("mime-type", mime_type)
            .property("created-at", created_at)
            .property("asset-type", asset_type)
            .property("sync-state", 1u32)
            .property("thumbhash", None::<String>)
            .property("local-path", local_path)
            .property("remote-id", "")
            .property("width", 0u32)
            .property("height", 0u32)
            .build()
    }
}
