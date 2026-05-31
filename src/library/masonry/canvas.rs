//! Justified-row masonry layout for the photos grid.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::graphene::{Rect, Size};
use gtk::gsk::RoundedRect;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::grid_view::AssetContextMenuHandler;
use crate::api_client::ThumbnailSize;
use crate::library::asset_model::LibraryAssetModel;
use crate::library::asset_object::AssetObject;
use crate::library::thumbnail_cache::ThumbnailCache;

type ActivateHandler = Rc<dyn Fn(u32)>;
type SelectModeChanger = Rc<dyn Fn(bool)>;

pub(super) const CORNER_RADIUS: f32 = 0.0;

const VIEWPORTS_BEHIND: f32 = 2.0;
const VIEWPORTS_AHEAD: f32 = 4.0;

use super::layout::{
    LaidItem, LaidRow, LayoutConfig, first_row_at_or_after, item_at_x, pack_rows, row_at_y,
};
pub use super::quality::GridQuality;
use super::quality::{bucket_for_row_height, fallback_bucket};

mod interaction;

mod imp {
    use super::*;

    pub(super) enum PaintResult {
        Hit,
        Miss,
    }

    struct SnapshotCtx<'a> {
        model: &'a LibraryAssetModel,
        cache: &'a Arc<ThumbnailCache>,
        placeholder: gdk4::RGBA,
        select_tint: gdk4::RGBA,
        selection: Option<&'a gtk::MultiSelection>,
    }

    #[derive(Default)]
    pub(super) struct SnapshotStats {
        pub(super) hits: usize,
        pub(super) misses: usize,
        pub(super) painted: usize,
    }

    struct SnapshotViewport {
        scroll_y: f32,
        center: f32,
        top: f32,
        bottom: f32,
        band_top: f32,
        band_bottom: f32,
    }

    pub(super) struct LoadRequest {
        pub(super) priority: f32,
        pub(super) asset_id: String,
        pub(super) bucket: ThumbnailSize,
        pub(super) local_path: String,
        pub(super) is_local: bool,
    }

    #[derive(Default)]
    pub struct MasonryCanvas {
        pub model: OnceCell<LibraryAssetModel>,
        pub cache: OnceCell<Arc<ThumbnailCache>>,
        pub selection: OnceCell<gtk::MultiSelection>,
        pub narrow: Cell<bool>,
        pub select_mode: Cell<bool>,
        pub quality: Cell<GridQuality>,
        pub rows: RefCell<Vec<LaidRow>>,
        pub cached_width: Cell<f32>,
        pub layout_h: Cell<f32>,
        pub pending: RefCell<HashSet<String>>,
        /// Permanently failed ids; skipped to avoid re-queueing every frame.
        pub failed: RefCell<HashSet<String>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub activate_handler: RefCell<Option<ActivateHandler>>,
        pub context_menu_handler: RefCell<Option<AssetContextMenuHandler>>,
        pub select_mode_changer: RefCell<Option<SelectModeChanger>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MasonryCanvas {
        const NAME: &'static str = "MimickMasonryCanvas";
        type Type = super::MasonryCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for MasonryCanvas {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().add_css_class("mimick-masonry-canvas");
            self.cached_width.set(-1.0);
        }
    }

    impl WidgetImpl for MasonryCanvas {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (200, 200, -1, -1),
                _ => {
                    let width = if for_size > 0 {
                        for_size as f32
                    } else {
                        self.cached_width.get().max(200.0)
                    };
                    let h = self.layout_for_width(width);
                    let h_i = h.ceil() as i32;
                    (h_i, h_i, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let w = width.max(0) as f32;
            if (w - self.cached_width.get()).abs() > 0.5 {
                self.cached_width.set(-1.0);
            }
            let _ = self.layout_for_width(w);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let canvas_w = widget.width() as f32;
            if canvas_w <= 0.0 {
                return;
            }
            let _ = self.layout_for_width(canvas_w);

            let rows = self.rows.borrow();
            let Some(model) = self.model.get() else {
                return;
            };
            let Some(cache) = self.cache.get() else {
                return;
            };

            let viewport = self.snapshot_viewport();
            let sctx = self.snapshot_ctx(model, cache);
            let (stats, mut to_load) = self.paint_snapshot_rows(snapshot, &sctx, &rows, &viewport);
            drop(rows);
            self.finish_snapshot(cache, &viewport, &stats, &mut to_load);
        }
    }

    impl MasonryCanvas {
        fn cfg(&self) -> LayoutConfig {
            if self.narrow.get() {
                LayoutConfig::narrow()
            } else {
                LayoutConfig::wide()
            }
        }

        fn layout_for_width(&self, width: f32) -> f32 {
            let cached = self.cached_width.get();
            if (width - cached).abs() < 0.5 && cached >= 0.0 {
                return self.layout_h.get();
            }
            let Some(model) = self.model.get() else {
                self.cached_width.set(width);
                self.layout_h.set(0.0);
                return 0.0;
            };
            let dims = collect_dims(model);
            let (rows, h) = pack_rows(&dims, width, self.cfg());
            *self.rows.borrow_mut() = rows;
            self.cached_width.set(width);
            self.layout_h.set(h);
            h
        }

        pub(super) fn invalidate_layout(&self) {
            // Pending and textures are NOT cleared here — they are independent
            // of layout and are owned per-asset across reflows.
            self.cached_width.set(-1.0);
            self.layout_h.set(0.0);
            self.rows.borrow_mut().clear();
            self.obj().queue_resize();
        }

        fn snapshot_ctx<'a>(
            &'a self,
            model: &'a LibraryAssetModel,
            cache: &'a Arc<ThumbnailCache>,
        ) -> SnapshotCtx<'a> {
            SnapshotCtx {
                model,
                cache,
                placeholder: placeholder_color(),
                select_tint: gdk4::RGBA::new(0.30, 0.55, 0.95, 0.35),
                selection: self.selection.get(),
            }
        }

        fn snapshot_viewport(&self) -> SnapshotViewport {
            let (scroll_y, viewport_h) = self.viewport();
            SnapshotViewport {
                scroll_y,
                center: scroll_y + viewport_h * 0.5,
                top: scroll_y,
                bottom: scroll_y + viewport_h,
                band_top: scroll_y - viewport_h * VIEWPORTS_BEHIND,
                band_bottom: scroll_y + viewport_h * (1.0 + VIEWPORTS_AHEAD),
            }
        }

        fn paint_snapshot_rows(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            rows: &[LaidRow],
            viewport: &SnapshotViewport,
        ) -> (SnapshotStats, Vec<LoadRequest>) {
            let mut stats = SnapshotStats::default();
            let mut to_load = Vec::new();
            let start_idx = first_row_at_or_after(rows, viewport.band_top);

            for row in rows[start_idx..]
                .iter()
                .take_while(|row| row.y <= viewport.band_bottom)
            {
                self.paint_row(snapshot, sctx, row, viewport, &mut stats, &mut to_load);
            }
            (stats, to_load)
        }

        fn paint_row(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            row: &LaidRow,
            viewport: &SnapshotViewport,
            stats: &mut SnapshotStats,
            to_load: &mut Vec<LoadRequest>,
        ) {
            let row_in_viewport = row.y + row.h > viewport.top && row.y < viewport.bottom;
            let bucket = bucket_for_row_height(row.h, self.quality.get());

            for it in &row.items {
                stats.record(self.paint_item(snapshot, sctx, row, it, row_in_viewport));
                self.queue_load_if_needed(sctx.model, it, row, viewport.center, bucket, to_load);
            }
        }

        fn paint_item(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            row: &LaidRow,
            it: &LaidItem,
            row_in_viewport: bool,
        ) -> PaintResult {
            let asset = sctx
                .model
                .item(it.asset_index)
                .and_downcast::<AssetObject>();
            let rect = Rect::new(it.x, row.y, it.w, row.h);

            let Some(asset) = asset else {
                snapshot.append_color(&sctx.placeholder, &rect);
                return PaintResult::Miss;
            };

            let asset_id = asset.property::<String>("id");
            let local_path = asset.property::<String>("local-path");
            let bucket = bucket_for_asset(&asset_id, &local_path, row.h, self.quality.get());
            let cached = cached_texture(sctx.cache, &asset_id, bucket, row_in_viewport);

            let clipped = CORNER_RADIUS > 0.0;
            if clipped {
                let corner = Size::new(CORNER_RADIUS, CORNER_RADIUS);
                let rounded = RoundedRect::new(rect, corner, corner, corner, corner);
                snapshot.push_rounded_clip(&rounded);
            }

            let result = if let Some(tex) = cached.as_ref() {
                snapshot.append_texture(tex, &rect);
                PaintResult::Hit
            } else {
                snapshot.append_color(&sctx.placeholder, &rect);
                PaintResult::Miss
            };

            let selected = sctx
                .selection
                .map(|s| s.is_selected(it.asset_index))
                .unwrap_or(false);
            if selected {
                snapshot.append_color(&sctx.select_tint, &rect);
            }
            if clipped {
                snapshot.pop();
            }
            result
        }

        fn queue_load_if_needed(
            &self,
            model: &LibraryAssetModel,
            it: &LaidItem,
            row: &LaidRow,
            viewport_center: f32,
            bucket: ThumbnailSize,
            to_load: &mut Vec<LoadRequest>,
        ) {
            let Some((asset_id, local_path, is_local_only)) = load_target_for(model, it) else {
                return;
            };
            let lookup_bucket = if is_local_only {
                ThumbnailSize::Thumbnail
            } else {
                bucket
            };
            if self.load_is_blocked(&asset_id, lookup_bucket) {
                return;
            }
            let mut pending = self.pending.borrow_mut();
            if pending.contains(&asset_id) {
                return;
            }
            pending.insert(asset_id.clone());
            let cell_center = row.y + row.h * 0.5;
            let priority = (cell_center - viewport_center).abs();
            to_load.push(LoadRequest {
                priority,
                asset_id,
                bucket,
                local_path,
                is_local: is_local_only,
            });
        }

        fn load_is_blocked(&self, asset_id: &str, bucket: ThumbnailSize) -> bool {
            self.failed.borrow().contains(asset_id)
                || self
                    .cache
                    .get()
                    .is_none_or(|cache| cache.peek_cached(asset_id, bucket).is_some())
        }

        fn finish_snapshot(
            &self,
            cache: &ThumbnailCache,
            viewport: &SnapshotViewport,
            stats: &SnapshotStats,
            to_load: &mut Vec<LoadRequest>,
        ) {
            to_load.sort_by(|a, b| {
                a.priority
                    .partial_cmp(&b.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            self.log_snapshot(cache, viewport, stats, to_load.len());
            for request in to_load.drain(..) {
                self.spawn_load(request);
            }
        }

        fn log_snapshot(
            &self,
            cache: &ThumbnailCache,
            viewport: &SnapshotViewport,
            stats: &SnapshotStats,
            queued: usize,
        ) {
            let (cache_n, cache_bytes, cache_max, cache_evicts) = cache.cache_stats();
            log::debug!(
                "masonry snapshot y={:.0} painted={} hits={} misses={} pending={} queued={} cache={}/{}/{}MB evicts={}",
                viewport.scroll_y,
                stats.painted,
                stats.hits,
                stats.misses,
                self.pending.borrow().len(),
                queued,
                cache_n,
                cache_bytes / (1024 * 1024),
                cache_max / (1024 * 1024),
                cache_evicts,
            );
        }

        fn viewport(&self) -> (f32, f32) {
            let adj = self.find_vadjustment();
            if let Some(adj) = adj {
                (adj.value() as f32, adj.page_size() as f32)
            } else {
                (0.0, self.obj().height() as f32)
            }
        }

        fn find_vadjustment(&self) -> Option<gtk::Adjustment> {
            if let Some(adj) = self.vadjustment.borrow().clone() {
                return Some(adj);
            }
            let mut node: Option<gtk::Widget> = self.obj().parent();
            while let Some(w) = node {
                if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
                    let adj = sw.vadjustment();
                    let weak = self.obj().downgrade();
                    adj.connect_value_changed(move |_| {
                        if let Some(canvas) = weak.upgrade() {
                            canvas.queue_draw();
                        }
                    });
                    *self.vadjustment.borrow_mut() = Some(adj.clone());
                    return Some(adj);
                }
                node = w.parent();
            }
            None
        }

        fn spawn_load(&self, request: LoadRequest) {
            let Some(cache) = self.cache.get().cloned() else {
                return;
            };
            let Some(model) = self.model.get().cloned() else {
                return;
            };
            let widget = self.obj().clone();
            log::trace!(
                "masonry spawn_load id={} bucket={:?} local={}",
                request.asset_id,
                request.bucket,
                request.is_local
            );
            glib::MainContext::default().spawn_local(async move {
                let asset_id = request.asset_id.clone();
                let result = load_masonry_asset(&cache, &request).await;
                finish_masonry_load(&widget, &model, &asset_id, result);
            });
        }
    }
}

use super::load::{collect_dims, load_with_fallback, propagate_dimensions};

glib::wrapper! {
    pub struct MasonryCanvas(ObjectSubclass<imp::MasonryCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl imp::SnapshotStats {
    fn record(&mut self, result: imp::PaintResult) {
        match result {
            imp::PaintResult::Hit => self.hits += 1,
            imp::PaintResult::Miss => self.misses += 1,
        }
        self.painted += 1;
    }
}

fn placeholder_color() -> gdk4::RGBA {
    if libadwaita::StyleManager::default().is_dark() {
        gdk4::RGBA::new(0.20, 0.20, 0.22, 1.0)
    } else {
        gdk4::RGBA::new(0.90, 0.90, 0.92, 1.0)
    }
}

fn bucket_for_asset(
    asset_id: &str,
    local_path: &str,
    row_h: f32,
    quality: GridQuality,
) -> ThumbnailSize {
    if !local_path.is_empty() && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX) {
        ThumbnailSize::Thumbnail
    } else {
        bucket_for_row_height(row_h, quality)
    }
}

fn cached_texture(
    cache: &ThumbnailCache,
    asset_id: &str,
    bucket: ThumbnailSize,
    touch_lru: bool,
) -> Option<gdk4::Texture> {
    let get = |size| {
        if touch_lru {
            cache.get_cached(asset_id, size)
        } else {
            cache.peek_cached(asset_id, size)
        }
    };
    get(bucket).or_else(|| fallback_bucket(bucket).and_then(get))
}

fn load_target_for(model: &LibraryAssetModel, it: &LaidItem) -> Option<(String, String, bool)> {
    let asset = model.item(it.asset_index).and_downcast::<AssetObject>()?;
    let asset_id = asset.property::<String>("id");
    let local_path = asset.property::<String>("local-path");
    let is_local = !local_path.is_empty() && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX);
    Some((asset_id, local_path, is_local))
}

async fn load_masonry_asset(
    cache: &ThumbnailCache,
    request: &imp::LoadRequest,
) -> Result<gdk4::Texture, String> {
    let is_cancelled = || false;
    if request.is_local {
        let path = std::path::PathBuf::from(&request.local_path);
        cache
            .load_local_thumbnail_cancellable(&request.asset_id, &path, &is_cancelled)
            .await
    } else {
        load_with_fallback(cache, &request.asset_id, request.bucket, &is_cancelled).await
    }
}

fn finish_masonry_load(
    widget: &MasonryCanvas,
    model: &LibraryAssetModel,
    asset_id: &str,
    result: Result<gdk4::Texture, String>,
) {
    let imp = widget.imp();
    let dims_changed = match &result {
        Ok(tex) => {
            log::trace!(
                "masonry load OK id={} dims=({}x{})",
                asset_id,
                tex.width(),
                tex.height(),
            );
            propagate_dimensions(model, asset_id, tex)
        }
        Err(e) => {
            if e != "cancelled" {
                imp.failed.borrow_mut().insert(asset_id.to_string());
            }
            log::warn!("masonry load ERR id={} err={}", asset_id, e);
            false
        }
    };
    imp.pending.borrow_mut().remove(asset_id);
    if dims_changed {
        imp.invalidate_layout();
    }
    widget.queue_draw();
}
