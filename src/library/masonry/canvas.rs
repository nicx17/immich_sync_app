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
use libadwaita::prelude::*;

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
        /// Anchor position for Shift+Click range selection.
        pub last_selected: Cell<Option<u32>>,
        pub permission_warned: Cell<bool>,
        /// Cached video badge icon paintable, resolved lazily from the icon theme.
        pub video_icon: OnceCell<gdk4::Paintable>,
        /// Cached checkbox icon (selected state).
        pub check_icon: OnceCell<gdk4::Paintable>,
        /// Cached checkbox icon (unselected state).
        pub uncheck_icon: OnceCell<gdk4::Paintable>,
        /// True while a drag-out operation is in progress; suppresses click-to-activate.
        pub drag_active: Cell<bool>,
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
                gtk::Orientation::Horizontal => (120, 200, -1, -1),
                _ => {
                    let width = if for_size > 0 {
                        for_size as f32
                    } else {
                        self.cached_width.get().max(120.0)
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

    /// Compact layout threshold: canvas widths below this use the
    /// smallest tile tier for phone-sized screens.
    const COMPACT_WIDTH_THRESHOLD: f32 = 400.0;

    impl MasonryCanvas {
        fn cfg(&self) -> LayoutConfig {
            if self.narrow.get() {
                let w = self.cached_width.get();
                if w > 0.0 && w < COMPACT_WIDTH_THRESHOLD {
                    LayoutConfig::compact()
                } else {
                    LayoutConfig::narrow()
                }
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

            // Checkbox badge in select mode.
            if self.select_mode.get() {
                self.paint_checkbox(snapshot, it.x, row.y, row.h, selected);
            }

            // Video badge: paint a centred play icon over video asset cells.
            let is_video = asset
                .property::<String>("asset-type")
                .eq_ignore_ascii_case("VIDEO");
            if is_video {
                self.paint_video_badge(snapshot, it.x, row.y, it.w, row.h);
            }

            if clipped {
                snapshot.pop();
            }
            result
        }

        /// Paint the video play-icon badge centred on the cell.
        ///
        /// A semi-transparent dark circle is drawn behind the icon for contrast,
        /// then the cached `mimick-video-symbolic` icon is rendered on top.
        fn paint_video_badge(
            &self,
            snapshot: &gtk::Snapshot,
            cell_x: f32,
            cell_y: f32,
            cell_w: f32,
            cell_h: f32,
        ) {
            let icon = self
                .video_icon
                .get_or_init(|| resolve_video_icon(&self.obj()));

            let icon_size = (cell_h * 0.22).clamp(16.0, 48.0);
            let bg_size = icon_size * 1.6;

            // Centre the background circle.
            let bg_x = cell_x + (cell_w - bg_size) * 0.5;
            let bg_y = cell_y + (cell_h - bg_size) * 0.5;
            let bg_rect = Rect::new(bg_x, bg_y, bg_size, bg_size);
            let half = bg_size * 0.5;
            let corner = Size::new(half, half);
            let rounded = RoundedRect::new(bg_rect, corner, corner, corner, corner);

            let bg_color = gdk4::RGBA::new(0.0, 0.0, 0.0, 0.55);
            snapshot.push_rounded_clip(&rounded);
            snapshot.append_color(&bg_color, &bg_rect);
            snapshot.pop();

            // Centre the icon inside the circle.
            let icon_x = cell_x + (cell_w - icon_size) * 0.5;
            let icon_y = cell_y + (cell_h - icon_size) * 0.5;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(icon_x, icon_y));
            icon.snapshot(
                snapshot.upcast_ref::<gdk4::Snapshot>(),
                icon_size as f64,
                icon_size as f64,
            );
            snapshot.restore();
        }

        /// Paint a checkbox indicator in the top-left corner of a tile.
        fn paint_checkbox(
            &self,
            snapshot: &gtk::Snapshot,
            cell_x: f32,
            cell_y: f32,
            cell_h: f32,
            selected: bool,
        ) {
            let box_size = (cell_h * 0.14).clamp(16.0, 26.0);
            let margin = 6.0_f32;
            let bx = cell_x + margin;
            let by = cell_y + margin;
            if selected {
                self.paint_checked_box(snapshot, bx, by, box_size);
            } else {
                Self::paint_unchecked_box(snapshot, bx, by, box_size);
            }
        }

        /// Accent-filled square with a checkmark icon.
        fn paint_checked_box(&self, snapshot: &gtk::Snapshot, bx: f32, by: f32, box_size: f32) {
            let r = 4.0_f32;
            let corner = Size::new(r, r);
            let rect = Rect::new(bx, by, box_size, box_size);
            let rounded = RoundedRect::new(rect, corner, corner, corner, corner);
            snapshot.push_rounded_clip(&rounded);
            snapshot.append_color(&accent_bg_color(), &rect);
            snapshot.pop();

            let icon = self
                .check_icon
                .get_or_init(|| resolve_symbolic_icon(&self.obj(), "object-select-symbolic"));
            let inset = 3.0_f32;
            let icon_size = box_size - inset * 2.0;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(bx + inset, by + inset));
            icon.snapshot(
                snapshot.upcast_ref::<gdk4::Snapshot>(),
                icon_size as f64,
                icon_size as f64,
            );
            snapshot.restore();
        }

        /// Bordered empty square (outline only).
        fn paint_unchecked_box(snapshot: &gtk::Snapshot, bx: f32, by: f32, box_size: f32) {
            let r = 4.0_f32;
            let border_w = 2.0_f32;
            let corner = Size::new(r, r);
            let outer_rect = Rect::new(bx, by, box_size, box_size);
            let outer_round = RoundedRect::new(outer_rect, corner, corner, corner, corner);

            snapshot.push_rounded_clip(&outer_round);
            snapshot.append_color(&gdk4::RGBA::new(1.0, 1.0, 1.0, 0.8), &outer_rect);

            let ir = (r - border_w).max(0.0);
            let ic = Size::new(ir, ir);
            let inner = Rect::new(
                bx + border_w,
                by + border_w,
                box_size - border_w * 2.0,
                box_size - border_w * 2.0,
            );
            let inner_round = RoundedRect::new(inner, ic, ic, ic, ic);
            snapshot.push_rounded_clip(&inner_round);
            snapshot.append_color(&gdk4::RGBA::new(0.0, 0.0, 0.0, 0.3), &inner);
            snapshot.pop();
            snapshot.pop();
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

/// Get the current theme accent background colour from libadwaita.
fn accent_bg_color() -> gdk4::RGBA {
    let accent = libadwaita::StyleManager::default().accent_color();
    accent.to_rgba()
}

fn placeholder_color() -> gdk4::RGBA {
    if libadwaita::StyleManager::default().is_dark() {
        gdk4::RGBA::new(0.20, 0.20, 0.22, 1.0)
    } else {
        gdk4::RGBA::new(0.90, 0.90, 0.92, 1.0)
    }
}

/// Resolve the video badge icon from the icon theme. The result is cached
/// per-canvas via a `OnceCell` so the theme lookup only happens once.
fn resolve_video_icon(widget: &MasonryCanvas) -> gdk4::Paintable {
    resolve_symbolic_icon(widget, "mimick-video-symbolic")
}

fn resolve_symbolic_icon(widget: &MasonryCanvas, icon_name: &str) -> gdk4::Paintable {
    let display = widget
        .native()
        .map(|n| n.display())
        .unwrap_or_else(|| gtk::gdk::Display::default().expect("default GDK display"));
    let theme = gtk::IconTheme::for_display(&display);
    let icon = theme.lookup_icon(
        icon_name,
        &[],
        48,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::FORCE_SYMBOLIC,
    );
    icon.upcast::<gdk4::Paintable>()
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

                if (e.contains("HTTP 401") || e.contains("HTTP 403"))
                    && !imp.permission_warned.get()
                {
                    imp.permission_warned.set(true);
                    if let Some(window) = widget
                        .root()
                        .and_downcast::<libadwaita::ApplicationWindow>()
                    {
                        let dialog = libadwaita::AlertDialog::builder()
                            .heading("Missing API Permissions")
                            .body("Your API key is missing permissions required to view thumbnails. Please ensure the key has 'asset.view' enabled.")
                            .build();
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&window));
                    }
                }
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
