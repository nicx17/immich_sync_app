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

mod imp {
    use super::*;

    enum PaintResult {
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

            let (scroll_y, viewport_h) = self.viewport();
            let viewport_center = scroll_y + viewport_h * 0.5;
            let viewport_top = scroll_y;
            let viewport_bottom = scroll_y + viewport_h;
            let band_top = scroll_y - viewport_h * VIEWPORTS_BEHIND;
            let band_bottom = scroll_y + viewport_h * (1.0 + VIEWPORTS_AHEAD);

            let rows = self.rows.borrow();
            let Some(model) = self.model.get() else {
                return;
            };
            let Some(cache) = self.cache.get() else {
                return;
            };

            let sctx = SnapshotCtx {
                model,
                cache,
                placeholder: if libadwaita::StyleManager::default().is_dark() {
                    gdk4::RGBA::new(0.20, 0.20, 0.22, 1.0)
                } else {
                    gdk4::RGBA::new(0.90, 0.90, 0.92, 1.0)
                },
                select_tint: gdk4::RGBA::new(0.30, 0.55, 0.95, 0.35),
                selection: self.selection.get(),
            };
            let mut to_load: Vec<(f32, String, ThumbnailSize, String, bool)> = Vec::new();
            let mut hits = 0usize;
            let mut misses = 0usize;
            let mut painted = 0usize;
            let start_idx = first_row_at_or_after(&rows, band_top);

            for row in rows[start_idx..].iter() {
                if row.y > band_bottom {
                    break;
                }
                let row_in_viewport = row.y + row.h > viewport_top && row.y < viewport_bottom;
                for it in &row.items {
                    let item_painted = self.paint_item(snapshot, &sctx, row, it, row_in_viewport);
                    match item_painted {
                        PaintResult::Hit => hits += 1,
                        PaintResult::Miss => misses += 1,
                    }
                    painted += 1;

                    self.queue_load_if_needed(
                        model,
                        it,
                        row,
                        viewport_center,
                        bucket_for_row_height(row.h, self.quality.get()),
                        &mut to_load,
                    );
                }
            }
            drop(rows);
            to_load.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            let (cache_n, cache_bytes, cache_max, cache_evicts) = cache.cache_stats();
            log::debug!(
                "masonry snapshot y={:.0} painted={} hits={} misses={} pending={} queued={} cache={}/{}/{}MB evicts={}",
                scroll_y,
                painted,
                hits,
                misses,
                self.pending.borrow().len(),
                to_load.len(),
                cache_n,
                cache_bytes / (1024 * 1024),
                cache_max / (1024 * 1024),
                cache_evicts,
            );

            for (_, asset_id, bucket, local_path, is_local) in to_load {
                self.spawn_load(asset_id, bucket, local_path, is_local);
            }
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

        fn paint_item(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            row: &LaidRow,
            it: &LaidItem,
            row_in_viewport: bool,
        ) -> PaintResult {
            let bucket = bucket_for_row_height(row.h, self.quality.get());
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
            let is_local_only =
                !local_path.is_empty() && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX);

            let lookup_bucket = if is_local_only {
                ThumbnailSize::Thumbnail
            } else {
                bucket
            };
            let cached = if row_in_viewport {
                sctx.cache.get_cached(&asset_id, lookup_bucket).or_else(|| {
                    fallback_bucket(lookup_bucket)
                        .and_then(|fb| sctx.cache.get_cached(&asset_id, fb))
                })
            } else {
                sctx.cache
                    .peek_cached(&asset_id, lookup_bucket)
                    .or_else(|| {
                        fallback_bucket(lookup_bucket)
                            .and_then(|fb| sctx.cache.peek_cached(&asset_id, fb))
                    })
            };

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
            to_load: &mut Vec<(f32, String, ThumbnailSize, String, bool)>,
        ) {
            let Some(asset) = model.item(it.asset_index).and_downcast::<AssetObject>() else {
                return;
            };
            let asset_id = asset.property::<String>("id");
            if self.failed.borrow().contains(&asset_id) {
                return;
            }
            let Some(cache) = self.cache.get() else {
                return;
            };
            let local_path = asset.property::<String>("local-path");
            let is_local_only =
                !local_path.is_empty() && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX);
            let lookup_bucket = if is_local_only {
                ThumbnailSize::Thumbnail
            } else {
                bucket
            };
            if cache.peek_cached(&asset_id, lookup_bucket).is_some() {
                return;
            }
            let mut pending = self.pending.borrow_mut();
            if pending.contains(&asset_id) {
                return;
            }
            pending.insert(asset_id.clone());
            let cell_center = row.y + row.h * 0.5;
            let priority = (cell_center - viewport_center).abs();
            to_load.push((priority, asset_id, bucket, local_path, is_local_only));
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

        fn spawn_load(
            &self,
            asset_id: String,
            bucket: ThumbnailSize,
            local_path: String,
            is_local: bool,
        ) {
            let Some(cache) = self.cache.get().cloned() else {
                return;
            };
            let Some(model) = self.model.get().cloned() else {
                return;
            };
            let widget = self.obj().clone();
            let id_for_remove = asset_id.clone();
            let is_cancelled = || false;
            log::trace!(
                "masonry spawn_load id={} bucket={:?} local={}",
                asset_id,
                bucket,
                is_local
            );
            glib::MainContext::default().spawn_local(async move {
                let result = if is_local {
                    let path = std::path::PathBuf::from(&local_path);
                    cache
                        .load_local_thumbnail_cancellable(&asset_id, &path, &is_cancelled)
                        .await
                } else {
                    load_with_fallback(&cache, &asset_id, bucket, &is_cancelled).await
                };
                let imp = widget.imp();
                let mut dims_changed = false;
                match &result {
                    Ok(tex) => {
                        dims_changed = propagate_dimensions(&model, &asset_id, tex);
                        log::trace!(
                            "masonry load OK id={} dims=({}x{})",
                            asset_id,
                            tex.width(),
                            tex.height(),
                        );
                    }
                    Err(e) => {
                        if e != "cancelled" {
                            imp.failed.borrow_mut().insert(asset_id.clone());
                        }
                        log::warn!("masonry load ERR id={} err={}", asset_id, e);
                    }
                }
                imp.pending.borrow_mut().remove(&id_for_remove);
                if dims_changed {
                    imp.invalidate_layout();
                }
                widget.queue_draw();
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

impl Default for MasonryCanvas {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl MasonryCanvas {
    pub fn new(
        cache: Arc<ThumbnailCache>,
        model: LibraryAssetModel,
        selection: gtk::MultiSelection,
    ) -> Self {
        let canvas: Self = glib::Object::new();
        let imp = canvas.imp();
        let _ = imp.cache.set(cache);
        let _ = imp.model.set(model.clone());
        let _ = imp.selection.set(selection.clone());

        let weak = canvas.downgrade();
        model.connect_items_changed(move |model, position, removed, added| {
            if let Some(canvas) = weak.upgrade() {
                let imp = canvas.imp();
                let n = model.n_items();
                let mut current_ids: HashSet<String> = HashSet::with_capacity(n as usize);
                for i in 0..n {
                    if let Some(obj) = model.item(i).and_downcast::<AssetObject>() {
                        current_ids.insert(obj.property::<String>("id"));
                    }
                }
                imp.failed
                    .borrow_mut()
                    .retain(|id| current_ids.contains(id));
                log::debug!(
                    "masonry items_changed pos={} removed={} added={} model_n={}",
                    position,
                    removed,
                    added,
                    n,
                );
                if added == 0 && removed > 0 && n == 0 {
                    return;
                }
                imp.invalidate_layout();
                canvas.queue_draw();
            }
        });

        let weak = canvas.downgrade();
        selection.connect_selection_changed(move |_, _, _| {
            if let Some(canvas) = weak.upgrade() {
                canvas.queue_draw();
            }
        });

        canvas.install_gestures();
        canvas
    }

    pub fn set_narrow(&self, narrow: bool) {
        let imp = self.imp();
        if imp.narrow.replace(narrow) != narrow {
            imp.invalidate_layout();
            self.queue_draw();
        }
    }

    pub fn set_select_mode(&self, on: bool) {
        self.imp().select_mode.set(on);
    }

    pub fn set_quality(&self, quality: GridQuality) {
        let imp = self.imp();
        if imp.quality.replace(quality) != quality {
            self.queue_draw();
        }
    }

    pub fn set_activate_handler(&self, f: impl Fn(u32) + 'static) {
        *self.imp().activate_handler.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_context_menu_handler(&self, handler: AssetContextMenuHandler) {
        *self.imp().context_menu_handler.borrow_mut() = Some(handler);
    }

    pub fn set_select_mode_changer(&self, f: impl Fn(bool) + 'static) {
        *self.imp().select_mode_changer.borrow_mut() = Some(Rc::new(f));
    }

    fn hit_test(&self, x: f64, y: f64) -> Option<u32> {
        let rows = self.imp().rows.borrow();
        let r = row_at_y(&rows, y as f32)?;
        let row = &rows[r];
        item_at_x(row, x as f32).map(|it| it.asset_index)
    }

    fn handle_primary_click(&self, gesture: &gtk::GestureClick, x: f64, y: f64) {
        let Some(pos) = self.hit_test(x, y) else {
            return;
        };
        let imp = self.imp();
        let ctrl = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let Some(sel) = imp.selection.get() else {
            return;
        };

        if ctrl {
            toggle_selection(sel, pos);
            if !imp.select_mode.get()
                && let Some(changer) = imp.select_mode_changer.borrow().clone()
            {
                (*changer)(true);
            }
            return;
        }

        if imp.select_mode.get() {
            toggle_selection(sel, pos);
            return;
        }

        if let Some(handler) = imp.activate_handler.borrow().clone() {
            (*handler)(pos);
        }
    }

    fn handle_secondary_click(&self, x: f64, y: f64) {
        let Some(pos) = self.hit_test(x, y) else {
            return;
        };
        let imp = self.imp();
        if let Some(handler_cell) = imp.context_menu_handler.borrow().clone()
            && let Some(cb) = handler_cell.borrow().as_ref()
        {
            (cb)(pos, x, y);
        }
    }

    fn install_gestures(&self) {
        let primary = gtk::GestureClick::new();
        primary.set_button(gtk::gdk::BUTTON_PRIMARY);
        let weak = self.downgrade();
        primary.connect_pressed(move |gesture, _, x, y| {
            if let Some(canvas) = weak.upgrade() {
                canvas.handle_primary_click(gesture, x, y);
            }
        });
        self.add_controller(primary);

        let secondary = gtk::GestureClick::new();
        secondary.set_button(gtk::gdk::BUTTON_SECONDARY);
        let weak = self.downgrade();
        secondary.connect_pressed(move |_, _, x, y| {
            if let Some(canvas) = weak.upgrade() {
                canvas.handle_secondary_click(x, y);
            }
        });
        self.add_controller(secondary);
    }
}

fn toggle_selection(sel: &gtk::MultiSelection, pos: u32) {
    if sel.is_selected(pos) {
        sel.unselect_item(pos);
    } else {
        sel.select_item(pos, false);
    }
}
