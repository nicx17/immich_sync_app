use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;

use super::{
    AssetContextMenuHandler, GridQuality, LibraryAssetModel, MasonryCanvas, ThumbnailCache, imp,
    item_at_x, row_at_y,
};
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
            enable_select_mode_if_needed(imp, pos);
        } else if imp.select_mode.get() {
            toggle_selection(sel, pos);
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
    primary.connect_pressed(move |gesture, _, x, y| {
        if let Some(canvas) = weak.upgrade() {
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
