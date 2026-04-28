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

impl AssetObject {
    /// Create a new `AssetObject` with all fields populated.
    pub fn new(
        id: &str,
        filename: &str,
        mime_type: &str,
        created_at: &str,
        asset_type: &str,
        sync_state: u32,
        thumbhash: Option<&str>,
    ) -> Self {
        glib::Object::builder()
            .property("id", id)
            .property("filename", filename)
            .property("mime-type", mime_type)
            .property("created-at", created_at)
            .property("asset-type", asset_type)
            .property("sync-state", sync_state)
            .property("thumbhash", thumbhash)
            .build()
    }
}
