//! Photos grid construction. Builds a `MasonryCanvas` inside a
//! `ScrolledWindow`, returns it alongside the shared model + selection.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;

use super::canvas::{GridQuality, MasonryCanvas};
use crate::app_context::AppContext;
use crate::library::asset_model::LibraryAssetModel;

pub type AssetContextMenuHandler = Rc<RefCell<Option<Box<dyn Fn(u32, f64, f64)>>>>;

pub struct GridViewParts {
    pub model: LibraryAssetModel,
    pub scrolled: gtk::ScrolledWindow,
    pub canvas: MasonryCanvas,
    pub selection: gtk::MultiSelection,
    pub context_menu_handler: AssetContextMenuHandler,
}

pub fn build_grid_view(
    ctx: Arc<AppContext>,
    select_toggle: gtk::ToggleButton,
    narrow: Rc<Cell<bool>>,
) -> GridViewParts {
    let model = LibraryAssetModel::new();
    let selection = gtk::MultiSelection::new(Some(model.clone()));
    let context_menu_handler: AssetContextMenuHandler = Rc::new(RefCell::new(None));

    let canvas = MasonryCanvas::new(
        ctx.thumbnail_cache.clone(),
        model.clone(),
        selection.clone(),
    );
    canvas.set_narrow(narrow.get());
    canvas.set_select_mode(select_toggle.is_active());
    canvas.set_context_menu_handler(context_menu_handler.clone());
    let initial_quality = GridQuality::parse(&ctx.config.read().data.library_grid_quality);
    canvas.set_quality(initial_quality);

    {
        let canvas = canvas.clone();
        select_toggle.connect_toggled(move |toggle| {
            canvas.set_select_mode(toggle.is_active());
        });
    }
    {
        let canvas = canvas.clone();
        let toggle = select_toggle.clone();
        canvas.set_select_mode_changer(move |on| {
            if toggle.is_active() != on {
                toggle.set_active(on);
            }
        });
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&canvas)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();

    GridViewParts {
        model,
        scrolled,
        canvas,
        selection,
        context_menu_handler,
    }
}
