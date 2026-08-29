use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;

use super::{
    AssetContextMenuHandler, GridQuality, LibraryAssetModel, MasonryCanvas, ThumbnailCache, imp,
    item_at_x, row_at_y,
};
use crate::app_context::AppContext;
use crate::library::asset_object::AssetObject;

impl Default for MasonryCanvas {
    fn default() -> Self {
        gtk::glib::Object::new()
    }
}

impl MasonryCanvas {
    pub fn new(
        cache: Arc<ThumbnailCache>,
        model: LibraryAssetModel,
        selection: gtk::MultiSelection,
    ) -> Self {
        let canvas: Self = gtk::glib::Object::new();
        let imp = canvas.imp();
        let _ = imp.cache.set(cache);
        let _ = imp.model.set(model.clone());
        let _ = imp.selection.set(selection.clone());

        connect_model_changes(&canvas, &model);
        connect_selection_changes(&canvas, &selection);
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

    pub fn set_border_gap(&self, gap: f32) {
        let imp = self.imp();
        if (imp.border_gap.get() - gap).abs() > f32::EPSILON {
            imp.border_gap.set(gap);
            imp.invalidate_layout();
            self.queue_draw();
        }
    }

    pub fn set_border_color(&self, color: gdk4::RGBA) {
        let imp = self.imp();
        imp.border_color.set(Some(color));
        if imp.border_gap.get() > 0.0 {
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
        let mods = gesture.current_event_state();
        let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let Some(sel) = imp.selection.get() else {
            return;
        };

        if shift {
            // Shift+Click: select contiguous range [anchor..pos].
            let anchor = imp.last_selected.get().unwrap_or(pos);
            let lo = anchor.min(pos);
            let hi = anchor.max(pos);
            // Shift only: replace selection with range. Ctrl+Shift: union.
            if !ctrl {
                sel.unselect_all();
            }
            select_range(sel, lo, hi);
            enable_select_mode_if_needed(imp, pos);
            // Preserve anchor so repeated Shift+Clicks extend from the same origin.
        } else if ctrl {
            toggle_selection(sel, pos);
            enable_select_mode_if_needed(imp, pos);
            imp.last_selected.set(Some(pos));
        } else if imp.select_mode.get() {
            toggle_selection(sel, pos);
            imp.last_selected.set(Some(pos));
        } else if let Some(handler) = imp.activate_handler.borrow().clone() {
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
        self.add_controller(primary_click_controller(self));
        self.add_controller(secondary_click_controller(self));
    }

    /// Install a `DragSource` for exporting original asset files via drag.
    ///
    /// Must be called after `new()` once the thumbnail cache is set up.
    /// The drag source resolves the original file from either the local path
    /// (for local/synced assets) or the lightbox preview cache (for remote assets
    /// that have been viewed in full resolution).
    pub fn install_drag_source(&self, ctx: Arc<AppContext>) {
        let drag_source = gtk::DragSource::new();
        drag_source.set_actions(gtk::gdk::DragAction::COPY);

        let weak = self.downgrade();
        let ctx_prepare = ctx.clone();
        drag_source.connect_prepare(move |_source, x, y| {
            let canvas = weak.upgrade()?;
            let files = collect_drag_files(&canvas, x, y, &ctx_prepare)?;
            Some(files_to_content_provider(files))
        });

        let weak2 = self.downgrade();
        drag_source.connect_drag_begin(move |source, _drag| {
            let Some(canvas) = weak2.upgrade() else {
                return;
            };
            canvas.imp().drag_active.set(true);
            let count = selected_count(&canvas);
            if count > 1 {
                source.set_icon(Some(&build_drag_badge(count)), 0, 0);
            }
        });

        let weak3 = self.downgrade();
        drag_source.connect_drag_end(move |_, _, _| {
            if let Some(canvas) = weak3.upgrade() {
                canvas.imp().drag_active.set(false);
            }
        });

        self.add_controller(drag_source);
    }
}

/// Collect gio::Files for the current drag operation (multi-select or single).
fn collect_drag_files(
    canvas: &MasonryCanvas,
    x: f64,
    y: f64,
    ctx: &AppContext,
) -> Option<Vec<gtk::gio::File>> {
    let imp = canvas.imp();
    let sel = imp.selection.get()?;
    let model = imp.model.get()?;
    let mut files = Vec::new();

    if imp.select_mode.get() {
        for i in 0..sel.n_items() {
            if sel.is_selected(i)
                && let Some(path) = resolve_drag_path(model, i, ctx)
            {
                files.push(gtk::gio::File::for_path(&path));
            }
        }
    }

    if files.is_empty() {
        let pos = canvas.hit_test(x, y)?;
        let path = resolve_drag_path(model, pos, ctx)?;
        files.push(gtk::gio::File::for_path(&path));
    }

    if files.is_empty() { None } else { Some(files) }
}

fn files_to_content_provider(files: Vec<gtk::gio::File>) -> gtk::gdk::ContentProvider {
    if files.len() == 1 {
        gtk::gdk::ContentProvider::for_value(&files[0].to_value())
    } else {
        let file_list = gtk::gdk::FileList::from_array(&files);
        gtk::gdk::ContentProvider::for_value(&file_list.to_value())
    }
}

/// Count selected items, or 1 if not in select mode.
fn selected_count(canvas: &MasonryCanvas) -> u32 {
    let imp = canvas.imp();
    if !imp.select_mode.get() {
        return 1;
    }
    imp.selection
        .get()
        .map(|s| (0..s.n_items()).filter(|i| s.is_selected(*i)).count() as u32)
        .unwrap_or(1)
}

/// Build a badge paintable showing the drag count.
fn build_drag_badge(count: u32) -> gtk::WidgetPaintable {
    let label = gtk::Label::builder()
        .label(format!("{count} files"))
        .css_classes(["mimick-drag-badge"])
        .build();
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["mimick-drag-badge"])
        .build();
    container.append(&label);
    gtk::WidgetPaintable::new(Some(&container))
}

fn connect_model_changes(canvas: &MasonryCanvas, model: &LibraryAssetModel) {
    let weak = canvas.downgrade();
    model.connect_items_changed(move |model, position, removed, added| {
        if let Some(canvas) = weak.upgrade() {
            handle_model_changed(&canvas, model, position, removed, added);
        }
    });
}

fn connect_selection_changes(canvas: &MasonryCanvas, selection: &gtk::MultiSelection) {
    let weak = canvas.downgrade();
    selection.connect_selection_changed(move |_, _, _| {
        if let Some(canvas) = weak.upgrade() {
            canvas.queue_draw();
        }
    });
}

fn handle_model_changed(
    canvas: &MasonryCanvas,
    model: &LibraryAssetModel,
    position: u32,
    removed: u32,
    added: u32,
) {
    let imp = canvas.imp();
    retain_failed_ids_for_model(imp, model);
    log::debug!(
        "masonry items_changed pos={} removed={} added={} model_n={}",
        position,
        removed,
        added,
        model.n_items(),
    );
    if added == 0 && removed > 0 && model.n_items() == 0 {
        return;
    }
    imp.invalidate_layout();
    canvas.queue_draw();
}

fn retain_failed_ids_for_model(imp: &imp::MasonryCanvas, model: &LibraryAssetModel) {
    let mut current_ids: HashSet<String> = HashSet::with_capacity(model.n_items() as usize);
    for i in 0..model.n_items() {
        if let Some(obj) = model.item(i).and_downcast::<AssetObject>() {
            current_ids.insert(obj.property::<String>("id"));
        }
    }
    imp.failed
        .borrow_mut()
        .retain(|id| current_ids.contains(id));
}

fn enable_select_mode_if_needed(imp: &imp::MasonryCanvas, _pos: u32) {
    if !imp.select_mode.get()
        && let Some(changer) = imp.select_mode_changer.borrow().clone()
    {
        (*changer)(true);
    }
}

fn primary_click_controller(canvas: &MasonryCanvas) -> gtk::GestureClick {
    let primary = gtk::GestureClick::new();
    primary.set_button(gtk::gdk::BUTTON_PRIMARY);
    let weak = canvas.downgrade();
    // Use `released` instead of `pressed` so a drag gesture that starts
    // from the same press doesn't also trigger lightbox activation.
    primary.connect_released(move |gesture, _, x, y| {
        if let Some(canvas) = weak.upgrade() {
            if canvas.imp().drag_active.get() {
                return;
            }
            canvas.handle_primary_click(gesture, x, y);
        }
    });
    primary
}

fn secondary_click_controller(canvas: &MasonryCanvas) -> gtk::GestureClick {
    let secondary = gtk::GestureClick::new();
    secondary.set_button(gtk::gdk::BUTTON_SECONDARY);
    let weak = canvas.downgrade();
    secondary.connect_pressed(move |_, _, x, y| {
        if let Some(canvas) = weak.upgrade() {
            canvas.handle_secondary_click(x, y);
        }
    });
    secondary
}

fn toggle_selection(sel: &gtk::MultiSelection, pos: u32) {
    if sel.is_selected(pos) {
        sel.unselect_item(pos);
    } else {
        sel.select_item(pos, false);
    }
}

fn select_range(sel: &gtk::MultiSelection, lo: u32, hi: u32) {
    let count = hi - lo + 1;
    sel.select_range(lo, count, false);
}

/// Resolve the drag-export path for the asset at `pos`.
///
/// Priority: local path > drag export cache (with original filename)
///         > lightbox preview cache (hardlinked to proper name)
///         > on-demand download.
fn resolve_drag_path(
    model: &LibraryAssetModel,
    pos: u32,
    ctx: &AppContext,
) -> Option<std::path::PathBuf> {
    let obj = model.item(pos).and_downcast::<AssetObject>()?;
    let local_path = obj.property::<String>("local-path");
    if !local_path.is_empty() {
        let p = std::path::PathBuf::from(&local_path);
        if p.exists() {
            return Some(p);
        }
    }

    let remote_id = obj.property::<String>("remote-id");
    let filename = obj.property::<String>("filename");
    if remote_id.is_empty() || filename.is_empty() {
        return None;
    }

    let export_path = export_cache_path(&remote_id, &filename)?;
    if export_path.exists() {
        return Some(export_path);
    }

    if try_link_from_preview(&remote_id, &filename, &export_path) {
        return Some(export_path);
    }

    if download_for_export(ctx, &remote_id, &filename, &export_path) {
        Some(export_path)
    } else {
        None
    }
}

fn export_cache_path(remote_id: &str, filename: &str) -> Option<std::path::PathBuf> {
    let export_dir = crate::profile::cache_dir()?.join("drag_export");
    let _ = std::fs::create_dir_all(&export_dir);
    let safe_name = format!("{}_{}", &remote_id[..8.min(remote_id.len())], filename);
    Some(export_dir.join(&safe_name))
}

fn try_link_from_preview(remote_id: &str, filename: &str, export_path: &std::path::Path) -> bool {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|e| !e.is_empty())
        .unwrap_or("bin");
    let preview_dir = match crate::profile::cache_dir() {
        Some(p) => p.join("preview"),
        None => return false,
    };
    let cached = preview_dir.join(format!("{remote_id}.{ext}"));

    cached.exists()
        && (std::fs::hard_link(&cached, export_path).is_ok()
            || std::fs::copy(&cached, export_path).is_ok())
}

fn download_for_export(
    ctx: &AppContext,
    remote_id: &str,
    filename: &str,
    export_path: &std::path::Path,
) -> bool {
    log::debug!("Drag export: downloading {} ({})", remote_id, filename);
    let api = ctx.api_client.clone();
    let id = remote_id.to_string();
    let dest = export_path.to_path_buf();

    let downloaded = std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("Failed to create tokio runtime for drag download: {}", e);
                return false;
            }
        };
        match rt.block_on(api.download_original_to_file(&id, &dest, None)) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("Drag export download failed for {}: {}", id, e);
                let _ = std::fs::remove_file(&dest);
                false
            }
        }
    })
    .join()
    .unwrap_or(false);

    downloaded && export_path.exists()
}
