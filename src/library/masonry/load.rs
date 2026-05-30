//! Async load plumbing for the masonry grid.
//!
//! `load_with_fallback` consults the shared `ThumbnailCache`, walking the
//! `quality::fallback_bucket` chain on 404 so opt-in server features (e.g.
//! fullsize generation) degrade gracefully to whatever the server has.
//! `collect_dims` / `propagate_dimensions` keep the layout's `(w, h)` view
//! of each asset in sync with what eventually decodes.

use gdk4::Texture;
use gtk::prelude::*;

use crate::api_client::ThumbnailSize;
use crate::library::asset_model::LibraryAssetModel;
use crate::library::asset_object::AssetObject;
use crate::library::masonry::quality::fallback_bucket;
use crate::library::thumbnail_cache::ThumbnailCache;

pub(crate) fn collect_dims(model: &LibraryAssetModel) -> Vec<(u32, u32)> {
    let n = model.n_items();
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        if let Some(obj) = model.item(i).and_downcast::<AssetObject>() {
            out.push((obj.property::<u32>("width"), obj.property::<u32>("height")));
        } else {
            out.push((0, 0));
        }
    }
    out
}

/// Try requested bucket, then walk `fallback_bucket` chain on 404. Only the
/// 404 path is retried — auth, network, etc. surface unchanged.
pub(crate) async fn load_with_fallback<F: Fn() -> bool>(
    cache: &ThumbnailCache,
    asset_id: &str,
    requested: ThumbnailSize,
    is_cancelled: &F,
) -> Result<Texture, String> {
    let mut current = requested;
    loop {
        match cache
            .load_thumbnail_cancellable(asset_id, current, is_cancelled)
            .await
        {
            Ok(tex) => return Ok(tex),
            Err(e) if e.contains("404") => match fallback_bucket(current) {
                Some(next) => {
                    log::debug!(
                        "masonry fallback id={} {:?} -> {:?} ({})",
                        asset_id,
                        current,
                        next,
                        e
                    );
                    current = next;
                }
                None => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
}

/// Returns true if the AssetObject dimensions were filled in (relayout needed).
pub(crate) fn propagate_dimensions(
    model: &LibraryAssetModel,
    asset_id: &str,
    tex: &Texture,
) -> bool {
    let n = model.n_items();
    for i in 0..n {
        if let Some(obj) = model.item(i).and_downcast::<AssetObject>()
            && obj.property::<String>("id") == asset_id
        {
            let w = obj.property::<u32>("width");
            let h = obj.property::<u32>("height");
            if w == 0 || h == 0 {
                obj.set_property("width", tex.width() as u32);
                obj.set_property("height", tex.height() as u32);
                return true;
            }
            return false;
        }
    }
    false
}
