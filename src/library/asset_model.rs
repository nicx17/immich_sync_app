//! Custom `gio::ListModel` backing the library grid.
//!
//! Replaces the previous `gio::ListStore`-of-`AssetObject` mirror. The model
//! owns a `Vec<AssetObject>` reconciled from `LibraryState.assets`.
//! `extend` is used for append-pagination (server-side date sort lets us emit
//! a precise `items_changed(prev_n, 0, added)` so the visible viewport doesn't
//! rebind); `reset` is used for source switches and client-side re-sorts.

use std::cell::RefCell;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::api_client::LibraryAsset;
use crate::app_context::AppContext;
use crate::library::asset_object::{AssetInit, AssetObject};
use crate::library::state::LibrarySortMode;

mod imp {
    use super::*;
    use gio::subclass::prelude::ListModelImpl;

    #[derive(Default)]
    pub struct LibraryAssetModel {
        pub items: RefCell<Vec<AssetObject>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LibraryAssetModel {
        const NAME: &'static str = "MimickLibraryAssetModel";
        type Type = super::LibraryAssetModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for LibraryAssetModel {}

    impl ListModelImpl for LibraryAssetModel {
        fn item_type(&self) -> glib::Type {
            AssetObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.items.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.items
                .borrow()
                .get(position as usize)
                .map(|o| o.clone().upcast())
        }
    }
}

glib::wrapper! {
    pub struct LibraryAssetModel(ObjectSubclass<imp::LibraryAssetModel>) @implements gio::ListModel;
}

impl Default for LibraryAssetModel {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryAssetModel {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Replace all items and emit a `items_changed(0, prev, new)` reset.
    /// Use when the caller can't promise that existing positions are stable
    /// (source switch, client-side sort change, dedup that affected the head).
    pub fn reset(&self, ctx: &AppContext, assets: &[LibraryAsset], sort_mode: &LibrarySortMode) {
        let prev_n = self.imp().items.borrow().len() as u32;
        let new_items = build_sorted_asset_objects(assets, ctx, sort_mode);
        let new_n = new_items.len() as u32;
        *self.imp().items.borrow_mut() = new_items;
        self.items_changed(0, prev_n, new_n);
    }

    /// Append-style update for paginated loads. For server-side date sort
    /// (`NewestFirst`/`OldestFirst`) the existing positions are stable so we
    /// emit a precise tail-only `items_changed(prev_n, 0, added)`. For
    /// client-side sort modes the entire vec may have re-ordered, so we fall
    /// back to a full reset.
    pub fn extend(&self, ctx: &AppContext, assets: &[LibraryAsset], sort_mode: &LibrarySortMode) {
        let prev_n = self.imp().items.borrow().len() as u32;
        let new_items = build_sorted_asset_objects(assets, ctx, sort_mode);
        let new_n = new_items.len() as u32;
        let server_sorted = matches!(
            sort_mode,
            LibrarySortMode::NewestFirst | LibrarySortMode::OldestFirst
        );

        *self.imp().items.borrow_mut() = new_items;

        if server_sorted && new_n >= prev_n {
            let added = new_n - prev_n;
            if added > 0 {
                self.items_changed(prev_n, 0, added);
            }
        } else {
            self.items_changed(0, prev_n, new_n);
        }
    }
}

fn build_sorted_asset_objects(
    assets: &[LibraryAsset],
    ctx: &AppContext,
    sort_mode: &LibrarySortMode,
) -> Vec<AssetObject> {
    let mut items = build_asset_objects(assets, ctx);
    match sort_mode {
        LibrarySortMode::NewestFirst | LibrarySortMode::OldestFirst => {}
        LibrarySortMode::Filename => items.sort_by_cached_key(|o| {
            (
                o.property::<String>("filename").to_ascii_lowercase(),
                o.property::<String>("id"),
            )
        }),
        LibrarySortMode::FileType => items.sort_by_cached_key(|o| {
            (
                o.property::<String>("mime-type"),
                o.property::<String>("filename"),
                o.property::<String>("id"),
            )
        }),
    }
    items
}

fn build_asset_objects(assets: &[LibraryAsset], ctx: &AppContext) -> Vec<AssetObject> {
    use super::{LOCAL_ID_PREFIX, immich_checksum_to_hex};
    use crate::library::local_source::local_sync_state;

    assets
        .iter()
        .map(|asset| {
            if let Some(local_path) = asset.id.strip_prefix(LOCAL_ID_PREFIX) {
                let sync_state =
                    local_sync_state(&ctx.sync_index, std::path::Path::new(local_path));
                let object = AssetObject::new_local(
                    &asset.id,
                    &asset.filename,
                    &asset.mime_type,
                    &asset.created_at,
                    &asset.asset_type,
                    local_path,
                );
                if sync_state != 1 {
                    object.set_property("sync-state", sync_state);
                }
                return object;
            }
            let local_match = asset
                .checksum
                .as_deref()
                .and_then(immich_checksum_to_hex)
                .as_deref()
                .and_then(|hex| ctx.sync_index.local_path_for_checksum(hex));
            let sync_state = if local_match.is_some() { 2 } else { 0 };
            let (exif_w, exif_h) = asset
                .exif_info
                .as_ref()
                .map(|e| (e.exif_image_width, e.exif_image_height))
                .unwrap_or((None, None));
            let width = asset
                .width
                .map(|v| v.max(0.0) as u32)
                .filter(|v| *v > 0)
                .or(exif_w)
                .unwrap_or(0);
            let height = asset
                .height
                .map(|v| v.max(0.0) as u32)
                .filter(|v| *v > 0)
                .or(exif_h)
                .unwrap_or(0);
            let object = AssetObject::new(AssetInit {
                id: &asset.id,
                filename: &asset.filename,
                mime_type: &asset.mime_type,
                created_at: &asset.created_at,
                asset_type: &asset.asset_type,
                sync_state,
                thumbhash: asset.thumbhash.as_deref(),
                width,
                height,
            });
            if let Some(path) = local_match {
                object.set_property("local-path", path);
            }
            object
        })
        .collect()
}
