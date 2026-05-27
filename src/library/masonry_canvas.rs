//! Justified-row masonry layout for the photos grid.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gdk4::Texture;
use gtk::glib;
use gtk::graphene::{Rect, Size};
use gtk::gsk::RoundedRect;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::api_client::ThumbnailSize;
use crate::library::asset_model::LibraryAssetModel;
use crate::library::asset_object::AssetObject;
use crate::library::grid_view::AssetContextMenuHandler;
use crate::library::thumbnail_cache::ThumbnailCache;

type ActivateHandler = Rc<dyn Fn(u32)>;
type SelectModeChanger = Rc<dyn Fn(bool)>;

const FALLBACK_W: f32 = 4.0;
const FALLBACK_H: f32 = 3.0;

pub(super) const MIN_ROW_HEIGHT_NARROW: f32 = 120.0;
pub(super) const MAX_ROW_HEIGHT_NARROW: f32 = 240.0;
pub(super) const MIN_ROW_HEIGHT_WIDE: f32 = 180.0;
pub(super) const MAX_ROW_HEIGHT_WIDE: f32 = 360.0;

pub(super) const GAP: f32 = 0.0;
pub(super) const CORNER_RADIUS: f32 = 0.0;

/// Row height above which we request the larger Preview thumbnail.
const PREVIEW_BUCKET_THRESHOLD: f32 = 280.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaidItem {
    pub asset_index: u32,
    pub x: f32,
    pub w: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaidRow {
    pub y: f32,
    pub h: f32,
    pub items: Vec<LaidItem>,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    pub min_row_height: f32,
    pub max_row_height: f32,
    pub gap: f32,
}

impl LayoutConfig {
    pub(super) fn narrow() -> Self {
        Self {
            min_row_height: MIN_ROW_HEIGHT_NARROW,
            max_row_height: MAX_ROW_HEIGHT_NARROW,
            gap: GAP,
        }
    }

    pub(super) fn wide() -> Self {
        Self {
            min_row_height: MIN_ROW_HEIGHT_WIDE,
            max_row_height: MAX_ROW_HEIGHT_WIDE,
            gap: GAP,
        }
    }
}

fn aspect(width: u32, height: u32) -> f32 {
    if width == 0 || height == 0 {
        FALLBACK_W / FALLBACK_H
    } else {
        (width as f32) / (height as f32)
    }
}

/// Greedy justified-row pack. `dims[i] = (w, h)` for asset i.
pub(super) fn pack_rows(
    dims: &[(u32, u32)],
    canvas_w: f32,
    cfg: LayoutConfig,
) -> (Vec<LaidRow>, f32) {
    if dims.is_empty() || canvas_w <= 0.0 {
        return (Vec::new(), 0.0);
    }

    let mut rows: Vec<LaidRow> = Vec::new();
    let mut y_cursor = 0.0_f32;
    let mut i = 0_usize;

    while i < dims.len() {
        let mut indices: Vec<usize> = Vec::new();
        let mut summed_w = 0.0_f32;
        while i < dims.len() {
            let w_at_max = aspect(dims[i].0, dims[i].1) * cfg.max_row_height;
            let gap_before = if indices.is_empty() { 0.0 } else { cfg.gap };
            if !indices.is_empty() && summed_w + gap_before + w_at_max > canvas_w {
                break;
            }
            indices.push(i);
            summed_w += w_at_max + gap_before;
            i += 1;
        }
        let last_row = i >= dims.len() && summed_w + cfg.gap < canvas_w;

        let mut row_h = scale_to_fit(&indices, dims, canvas_w, cfg);

        // Pop the trailing item if the row is too short — it spills to the next row.
        if indices.len() > 1 && row_h < cfg.min_row_height {
            let popped = indices.pop().unwrap();
            i = popped;
            row_h = scale_to_fit(&indices, dims, canvas_w, cfg);
        }

        // Filled rows keep their computed height so the row reaches canvas_w.
        // Only the underfilled trailing row is clamped.
        if last_row {
            row_h = row_h.clamp(cfg.min_row_height, cfg.max_row_height);
        }

        let mut placed = Vec::with_capacity(indices.len());
        let mut x_cursor = 0.0_f32;
        for &idx in &indices {
            let w = aspect(dims[idx].0, dims[idx].1) * row_h;
            placed.push(LaidItem {
                asset_index: idx as u32,
                x: x_cursor,
                w,
            });
            x_cursor += w + cfg.gap;
        }

        rows.push(LaidRow {
            y: y_cursor,
            h: row_h,
            items: placed,
        });
        y_cursor += row_h + cfg.gap;
    }

    let total_height = (y_cursor - cfg.gap).max(0.0);
    (rows, total_height)
}

fn scale_to_fit(indices: &[usize], dims: &[(u32, u32)], canvas_w: f32, cfg: LayoutConfig) -> f32 {
    let total_gap = if indices.len() > 1 {
        cfg.gap * (indices.len() as f32 - 1.0)
    } else {
        0.0
    };
    let sum: f32 = indices
        .iter()
        .map(|&idx| aspect(dims[idx].0, dims[idx].1) * cfg.max_row_height)
        .sum();
    if sum <= 0.0 {
        return cfg.max_row_height;
    }
    let scale = ((canvas_w - total_gap) / sum).max(0.0);
    cfg.max_row_height * scale
}

pub(super) fn row_at_y(rows: &[LaidRow], y: f32) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let mut lo = 0;
    let mut hi = rows.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let r = &rows[mid];
        if y < r.y {
            hi = mid;
        } else if y >= r.y + r.h {
            lo = mid + 1;
        } else {
            return Some(mid);
        }
    }
    None
}

pub(super) fn item_at_x(row: &LaidRow, x: f32) -> Option<&LaidItem> {
    row.items.iter().find(|it| x >= it.x && x < it.x + it.w)
}

fn bucket_for_row_height(h: f32) -> ThumbnailSize {
    if h <= PREVIEW_BUCKET_THRESHOLD {
        ThumbnailSize::Thumbnail
    } else {
        ThumbnailSize::Preview
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MasonryCanvas {
        pub model: OnceCell<LibraryAssetModel>,
        pub cache: OnceCell<Arc<ThumbnailCache>>,
        pub selection: OnceCell<gtk::MultiSelection>,
        pub narrow: Cell<bool>,
        pub select_mode: Cell<bool>,
        pub rows: RefCell<Vec<LaidRow>>,
        pub cached_width: Cell<f32>,
        pub layout_h: Cell<f32>,
        pub pending: RefCell<HashSet<String>>,
        /// Textures retained per asset so re-painting after scroll never has
        /// to re-fetch when the shared ThumbnailCache LRU evicts.
        pub textures: RefCell<HashMap<String, Texture>>,
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
                gtk::Orientation::Horizontal => (0, 0, -1, -1),
                _ => {
                    let width = for_size.max(0) as f32;
                    let h = self.layout_for_width(width);
                    let h_i = h.ceil() as i32;
                    (h_i, h_i, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let _ = self.layout_for_width(width.max(0) as f32);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let canvas_w = widget.width() as f32;
            if canvas_w <= 0.0 {
                return;
            }
            let _ = self.layout_for_width(canvas_w);

            let (scroll_y, viewport_h) = self.viewport();
            let visible_top = scroll_y;
            let visible_bottom = scroll_y + viewport_h;

            let rows = self.rows.borrow();
            let Some(model) = self.model.get() else {
                return;
            };
            let Some(cache) = self.cache.get() else {
                return;
            };

            let placeholder = gdk4::RGBA::new(0.18, 0.18, 0.18, 1.0);
            let select_tint = gdk4::RGBA::new(0.30, 0.55, 0.95, 0.35);
            let selection = self.selection.get();
            let mut needs_load: Vec<(String, ThumbnailSize, String, bool)> = Vec::new();

            for row in rows.iter() {
                if row.y + row.h < visible_top {
                    continue;
                }
                if row.y > visible_bottom {
                    break;
                }
                for it in &row.items {
                    let rect = Rect::new(it.x, row.y, it.w, row.h);
                    let bucket = bucket_for_row_height(row.h);
                    let Some(asset) = model.item(it.asset_index).and_downcast::<AssetObject>()
                    else {
                        continue;
                    };
                    let asset_id = asset.property::<String>("id");
                    let local_path = asset.property::<String>("local-path");
                    let is_local_only = !local_path.is_empty()
                        && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX);

                    let cached = self.textures.borrow().get(&asset_id).cloned().or_else(|| {
                        if is_local_only {
                            None
                        } else {
                            cache.get_cached(&asset_id, bucket)
                        }
                    });
                    let clipped = CORNER_RADIUS > 0.0;
                    if clipped {
                        let corner = Size::new(CORNER_RADIUS, CORNER_RADIUS);
                        let rounded = RoundedRect::new(rect, corner, corner, corner, corner);
                        snapshot.push_rounded_clip(&rounded);
                    }
                    if let Some(tex) = cached {
                        snapshot.append_texture(&tex, &rect);
                    } else {
                        snapshot.append_color(&placeholder, &rect);
                        let mut pending = self.pending.borrow_mut();
                        if !pending.contains(&asset_id) {
                            pending.insert(asset_id.clone());
                            needs_load.push((asset_id, bucket, local_path, is_local_only));
                        }
                    }
                    let selected = selection
                        .map(|s| s.is_selected(it.asset_index))
                        .unwrap_or(false);
                    if selected {
                        snapshot.append_color(&select_tint, &rect);
                    }
                    if clipped {
                        snapshot.pop();
                    }
                }
            }
            drop(rows);

            for (asset_id, bucket, local_path, is_local) in needs_load {
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
            // Off-screen loads cancel by reading the live pending set: each
            // snapshot drops ids that are no longer visible, so backlogged
            // loads can release their semaphore permit.
            let cancel_widget = widget.downgrade();
            let cancel_id = asset_id.clone();
            let is_cancelled = move || {
                let Some(w) = cancel_widget.upgrade() else {
                    return true;
                };
                !w.imp().pending.borrow().contains(&cancel_id)
            };
            glib::MainContext::default().spawn_local(async move {
                let result = if is_local {
                    let path = std::path::PathBuf::from(&local_path);
                    cache
                        .load_local_thumbnail_cancellable(&asset_id, &path, is_cancelled)
                        .await
                } else {
                    cache
                        .load_thumbnail_cancellable(&asset_id, bucket, is_cancelled)
                        .await
                };
                let imp = widget.imp();
                let mut dims_changed = false;
                if let Ok(tex) = result {
                    dims_changed = propagate_dimensions(&model, &asset_id, &tex);
                    imp.textures.borrow_mut().insert(asset_id.clone(), tex);
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

fn collect_dims(model: &LibraryAssetModel) -> Vec<(u32, u32)> {
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

/// Returns true if the AssetObject dimensions were filled in (relayout needed).
fn propagate_dimensions(model: &LibraryAssetModel, asset_id: &str, tex: &Texture) -> bool {
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
        model.connect_items_changed(move |model, _, _, _| {
            if let Some(canvas) = weak.upgrade() {
                // Drop only textures whose asset_id is no longer in the model.
                // Avoids wiping retained textures on append-pagination that
                // emits a full-range items_changed (client-sorted modes).
                let imp = canvas.imp();
                let n = model.n_items();
                let mut current_ids: HashSet<String> = HashSet::with_capacity(n as usize);
                for i in 0..n {
                    if let Some(obj) = model.item(i).and_downcast::<AssetObject>() {
                        current_ids.insert(obj.property::<String>("id"));
                    }
                }
                imp.textures
                    .borrow_mut()
                    .retain(|id, _| current_ids.contains(id));
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

    fn install_gestures(&self) {
        let primary = gtk::GestureClick::new();
        primary.set_button(gtk::gdk::BUTTON_PRIMARY);
        let weak = self.downgrade();
        primary.connect_pressed(move |gesture, _, x, y| {
            let Some(canvas) = weak.upgrade() else {
                return;
            };
            let Some(pos) = canvas.hit_test(x, y) else {
                return;
            };
            let imp = canvas.imp();
            let ctrl = gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let Some(sel) = imp.selection.get() else {
                return;
            };

            if ctrl {
                if sel.is_selected(pos) {
                    sel.unselect_item(pos);
                } else {
                    sel.select_item(pos, false);
                }
                if !imp.select_mode.get()
                    && let Some(changer) = imp.select_mode_changer.borrow().clone()
                {
                    (*changer)(true);
                }
                return;
            }

            if imp.select_mode.get() {
                if sel.is_selected(pos) {
                    sel.unselect_item(pos);
                } else {
                    sel.select_item(pos, false);
                }
                return;
            }

            if let Some(handler) = imp.activate_handler.borrow().clone() {
                (*handler)(pos);
            }
        });
        self.add_controller(primary);

        let secondary = gtk::GestureClick::new();
        secondary.set_button(gtk::gdk::BUTTON_SECONDARY);
        let weak = self.downgrade();
        secondary.connect_pressed(move |_, _, x, y| {
            let Some(canvas) = weak.upgrade() else {
                return;
            };
            let Some(pos) = canvas.hit_test(x, y) else {
                return;
            };
            let imp = canvas.imp();
            if let Some(handler_cell) = imp.context_menu_handler.borrow().clone()
                && let Some(cb) = handler_cell.borrow().as_ref()
            {
                (cb)(pos, x, y);
            }
        });
        self.add_controller(secondary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LayoutConfig {
        LayoutConfig {
            min_row_height: 100.0,
            max_row_height: 200.0,
            gap: 0.0,
        }
    }

    #[test]
    fn empty_input_yields_empty_layout() {
        let (rows, h) = pack_rows(&[], 1000.0, cfg());
        assert!(rows.is_empty());
        assert_eq!(h, 0.0);
    }

    #[test]
    fn zero_canvas_width_yields_empty() {
        let (rows, h) = pack_rows(&[(100, 100)], 0.0, cfg());
        assert!(rows.is_empty());
        assert_eq!(h, 0.0);
    }

    #[test]
    fn fallback_aspect_when_dimensions_zero() {
        let (rows, _) = pack_rows(&[(0, 0), (0, 0), (0, 0)], 1200.0, cfg());
        assert_eq!(rows.len(), 1);
        assert!((rows[0].h - 200.0).abs() < 0.01);
    }

    #[test]
    fn full_row_fills_canvas_width_within_a_pixel() {
        let dims = &[(1600, 900), (1600, 900), (1600, 900), (1600, 900)];
        let (rows, _) = pack_rows(dims, 1200.0, cfg());
        let r1 = &rows[0];
        let last = r1.items.last().unwrap();
        let fill = last.x + last.w;
        assert!((fill - 1200.0).abs() < 1.0);
    }

    #[test]
    fn all_items_placed_across_rows() {
        let dims: Vec<(u32, u32)> = (0..8).map(|_| (3000, 1000)).collect();
        let (rows, _) = pack_rows(&dims, 1200.0, cfg());
        let total: usize = rows.iter().map(|r| r.items.len()).sum();
        assert_eq!(total, 8);
    }

    #[test]
    fn last_row_clamped_to_max_height() {
        let mut dims: Vec<(u32, u32)> = (0..6).map(|_| (4000, 3000)).collect();
        dims.push((1000, 1500));
        let (rows, _) = pack_rows(&dims, 1200.0, cfg());
        let last = rows.last().unwrap();
        assert!(last.h <= 200.0 + 0.01);
    }

    #[test]
    fn binary_search_finds_correct_row() {
        let rows = vec![
            LaidRow {
                y: 0.0,
                h: 100.0,
                items: vec![],
            },
            LaidRow {
                y: 100.0,
                h: 150.0,
                items: vec![],
            },
            LaidRow {
                y: 250.0,
                h: 80.0,
                items: vec![],
            },
        ];
        assert_eq!(row_at_y(&rows, 0.0), Some(0));
        assert_eq!(row_at_y(&rows, 100.0), Some(1));
        assert_eq!(row_at_y(&rows, 329.9), Some(2));
        assert_eq!(row_at_y(&rows, 330.0), None);
    }

    #[test]
    fn item_hit_test_within_row() {
        let row = LaidRow {
            y: 0.0,
            h: 100.0,
            items: vec![
                LaidItem {
                    asset_index: 5,
                    x: 0.0,
                    w: 50.0,
                },
                LaidItem {
                    asset_index: 6,
                    x: 50.0,
                    w: 80.0,
                },
                LaidItem {
                    asset_index: 7,
                    x: 130.0,
                    w: 40.0,
                },
            ],
        };
        assert_eq!(item_at_x(&row, 0.0).map(|i| i.asset_index), Some(5));
        assert_eq!(item_at_x(&row, 50.0).map(|i| i.asset_index), Some(6));
        assert_eq!(item_at_x(&row, 130.0).map(|i| i.asset_index), Some(7));
        assert!(item_at_x(&row, 200.0).is_none());
    }

    #[test]
    fn gap_increases_total_layout_height() {
        let dims = &[(100, 100), (100, 100), (100, 100), (100, 100)];
        let (_, h0) = pack_rows(dims, 200.0, LayoutConfig { gap: 0.0, ..cfg() });
        let (_, h1) = pack_rows(dims, 200.0, LayoutConfig { gap: 10.0, ..cfg() });
        assert!(h1 > h0);
    }

    #[test]
    fn bucket_thumbnail_under_threshold() {
        assert!(matches!(
            bucket_for_row_height(200.0),
            ThumbnailSize::Thumbnail
        ));
        assert!(matches!(
            bucket_for_row_height(280.0),
            ThumbnailSize::Thumbnail
        ));
        assert!(matches!(
            bucket_for_row_height(281.0),
            ThumbnailSize::Preview
        ));
    }
}
