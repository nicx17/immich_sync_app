//! GridView-based asset browser with async thumbnail loading and pagination.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gdk4::Texture;
use gtk::prelude::*;

use crate::app_context::AppContext;
use crate::library::asset_object::AssetObject;

const POS_DATA_KEY: &str = "mimick-cell-pos";

pub struct GridViewParts {
    pub model: gtk::gio::ListStore,
    pub scrolled: gtk::ScrolledWindow,
    pub view: gtk::GridView,
    pub selection: gtk::MultiSelection,
}

pub fn build_grid_view(ctx: Arc<AppContext>, select_toggle: gtk::ToggleButton) -> GridViewParts {
    let model = gtk::gio::ListStore::new::<AssetObject>();
    let selection = gtk::MultiSelection::new(Some(model.clone()));
    let factory = gtk::SignalListItemFactory::new();

    let select_toggle_for_setup = select_toggle.clone();
    let selection_for_setup = selection.clone();
    factory.connect_setup(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let container = gtk::Overlay::builder()
            .css_classes(vec!["mimick-cell".to_string()])
            .build();

        let picture = gtk::Picture::builder()
            .width_request(356)
            .height_request(200)
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Cover)
            .css_classes(vec!["mimick-thumbnail-loading".to_string()])
            .build();

        let checkbox = gtk::CheckButton::builder()
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .margin_start(6)
            .can_focus(false)
            .css_classes(vec!["mimick-select-checkbox".to_string()])
            .build();
        select_toggle_for_setup
            .bind_property("active", &checkbox, "visible")
            .sync_create()
            .build();

        let pos_cell = Rc::new(Cell::new(u32::MAX));
        let suppress = Rc::new(Cell::new(false));
        let pos_for_toggle = pos_cell.clone();
        let suppress_for_toggle = suppress.clone();
        let selection_for_toggle = selection_for_setup.clone();
        checkbox.connect_toggled(move |cb| {
            if suppress_for_toggle.get() {
                return;
            }
            let pos = pos_for_toggle.get();
            if pos == u32::MAX {
                return;
            }
            if cb.is_active() {
                selection_for_toggle.select_item(pos, false);
            } else {
                selection_for_toggle.unselect_item(pos);
            }
        });
        unsafe {
            checkbox.set_data::<(Rc<Cell<u32>>, Rc<Cell<bool>>)>(
                POS_DATA_KEY,
                (pos_cell.clone(), suppress.clone()),
            );
        }

        let cb_for_selection = checkbox.clone();
        let pos_for_selection = pos_cell.clone();
        let suppress_for_selection = suppress.clone();
        selection_for_setup.connect_selection_changed(move |sel, start, n_items| {
            let pos = pos_for_selection.get();
            if pos == u32::MAX || pos < start || pos >= start + n_items {
                return;
            }
            let selected = sel.is_selected(pos);
            if cb_for_selection.is_active() == selected {
                return;
            }
            suppress_for_selection.set(true);
            cb_for_selection.set_active(selected);
            suppress_for_selection.set(false);
        });

        let status = gtk::Image::builder()
            .icon_name("network-server-symbolic")
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .margin_end(6)
            .pixel_size(14)
            .css_classes(vec!["mimick-status-badge".to_string()])
            .build();

        container.set_child(Some(&picture));
        container.add_overlay(&checkbox);
        container.add_overlay(&status);
        unsafe {
            container.set_data::<Rc<Cell<u32>>>(POS_DATA_KEY, pos_cell);
        }
        list_item.set_child(Some(&container));
    });

    let selection_for_bind = selection.clone();
    factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let item = match list_item.item().and_downcast::<AssetObject>() {
            Some(item) => item,
            None => return,
        };
        let Some(container) = list_item.child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let Some(picture) = container.child().and_downcast::<gtk::Picture>() else {
            return;
        };
        let Some(status) = container.last_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let checkbox = find_select_checkbox(&container);

        let position = list_item.position();
        if let Some(cb) = checkbox.as_ref() {
            sync_checkbox_state(cb, position, selection_for_bind.is_selected(position));
        }

        let asset_id = item.property::<String>("id");
        let local_path = item.property::<String>("local-path");
        let sync_state = item.property::<u32>("sync-state");
        let thumbhash = item.property::<Option<String>>("thumbhash");
        picture.set_tooltip_text(Some(&asset_id));
        status.set_icon_name(Some(sync_icon_name(sync_state)));
        status.set_tooltip_text(Some(sync_state_label(sync_state)));

        let in_timeline = ctx
            .library_timeline_active
            .load(std::sync::atomic::Ordering::Relaxed);
        status.set_visible(!in_timeline);
        set_square_class(&picture, in_timeline);

        let generation = bump_generation(&picture);

        let cache = ctx.thumbnail_cache.clone();
        if let Some(texture) =
            cache.get_cached(&asset_id, crate::api_client::ThumbnailSize::Thumbnail)
        {
            picture.set_paintable(Some(&texture));
            set_thumb_state(&picture, ThumbState::Loaded);
            return;
        }

        match thumbhash.as_deref().and_then(decode_thumbhash_texture) {
            Some(placeholder) => {
                picture.set_paintable(Some(&placeholder));
                set_thumb_state(&picture, ThumbState::Loaded);
            }
            None => {
                picture.set_paintable(Option::<&Texture>::None);
                set_thumb_state(&picture, ThumbState::Loading);
            }
        }

        schedule_thumbnail_load(
            ctx.clone(),
            picture.clone(),
            asset_id,
            local_path,
            generation,
        );
    });

    factory.connect_unbind(|_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let Some(container) = list_item.child().and_downcast::<gtk::Overlay>()
            && let Some(picture) = container.child().and_downcast::<gtk::Picture>()
        {
            bump_generation(&picture);
            picture.set_tooltip_text(None);
            picture.set_paintable(Option::<&Texture>::None);
            set_thumb_state(&picture, ThumbState::Loading);
        }
    });

    let view = gtk::GridView::builder()
        .model(&selection)
        .factory(&factory)
        .single_click_activate(!select_toggle.is_active())
        .enable_rubberband(false)
        .max_columns(6)
        .min_columns(2)
        .build();
    select_toggle.connect_toggled(clone_view_for_toggle(&view));

    let ctrl_gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let selection_for_gesture = selection.clone();
    let select_toggle_for_gesture = select_toggle.clone();
    ctrl_gesture.connect_pressed(move |gesture, _n_press, x, y| {
        let state = gesture.current_event_state();
        if !state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            return;
        }
        let Some(view) = gesture.widget().and_downcast::<gtk::GridView>() else {
            return;
        };
        let Some(picked) = view.pick(x, y, gtk::PickFlags::DEFAULT) else {
            return;
        };
        let mut node = Some(picked);
        while let Some(widget) = node {
            if widget.has_css_class("mimick-cell") {
                let pos = unsafe {
                    widget
                        .data::<Rc<Cell<u32>>>(POS_DATA_KEY)
                        .map(|p| p.as_ref().get())
                };
                if let Some(pos) = pos
                    && pos != u32::MAX
                {
                    if selection_for_gesture.is_selected(pos) {
                        selection_for_gesture.unselect_item(pos);
                    } else {
                        selection_for_gesture.select_item(pos, false);
                    }
                    select_toggle_for_gesture.set_active(true);
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
                return;
            }
            node = widget.parent();
        }
    });
    view.add_controller(ctrl_gesture);
    if let Some(layout) = view.layout_manager().and_downcast::<gtk::GridLayout>() {
        layout.set_column_spacing(0);
        layout.set_row_spacing(0);
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();

    GridViewParts {
        model,
        scrolled,
        view,
        selection,
    }
}

fn clone_view_for_toggle(view: &gtk::GridView) -> impl Fn(&gtk::ToggleButton) + 'static {
    let view = view.clone();
    move |toggle| {
        view.set_single_click_activate(!toggle.is_active());
    }
}

fn find_select_checkbox(container: &gtk::Overlay) -> Option<gtk::CheckButton> {
    let mut child = container.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        if c.has_css_class("mimick-select-checkbox")
            && let Ok(cb) = c.downcast::<gtk::CheckButton>()
        {
            return Some(cb);
        }
        child = next;
    }
    None
}

fn sync_checkbox_state(checkbox: &gtk::CheckButton, position: u32, selected: bool) {
    let data = unsafe {
        checkbox
            .data::<(Rc<Cell<u32>>, Rc<Cell<bool>>)>(POS_DATA_KEY)
            .map(|p| p.as_ref().clone())
    };
    let Some((pos_cell, suppress)) = data else {
        return;
    };
    pos_cell.set(position);
    suppress.set(true);
    checkbox.set_active(selected);
    suppress.set(false);
}

pub fn replace_model(model: &gtk::gio::ListStore, objects: &[AssetObject]) {
    model.remove_all();
    for object in objects {
        model.append(object);
    }
}

pub fn extend_model(model: &gtk::gio::ListStore, objects: &[AssetObject]) {
    for object in objects {
        model.append(object);
    }
}

fn sync_state_label(sync_state: u32) -> &'static str {
    match sync_state {
        2 => "On Immich and locally",
        1 => "Local only",
        _ => "On Immich only",
    }
}

fn sync_icon_name(sync_state: u32) -> &'static str {
    match sync_state {
        2 => "emblem-default-symbolic",  // Both: solid check ✓
        1 => "folder-pictures-symbolic", // Local only
        _ => "network-server-symbolic",  // Remote only
    }
}

#[derive(Clone, Copy)]
enum ThumbState {
    Loading,
    Loaded,
    Error,
}

fn set_thumb_state(picture: &gtk::Picture, state: ThumbState) {
    let want = match state {
        ThumbState::Loading => "mimick-thumbnail-loading",
        ThumbState::Loaded => "mimick-thumbnail-loaded",
        ThumbState::Error => "mimick-thumbnail-error",
    };
    for cls in [
        "mimick-thumbnail-loading",
        "mimick-thumbnail-loaded",
        "mimick-thumbnail-error",
    ] {
        if cls == want {
            if !picture.has_css_class(cls) {
                picture.add_css_class(cls);
            }
        } else if picture.has_css_class(cls) {
            picture.remove_css_class(cls);
        }
    }
    if matches!(state, ThumbState::Error) {
        picture.set_tooltip_text(Some("Thumbnail unavailable"));
    }
}

fn set_square_class(picture: &gtk::Picture, on: bool) {
    let has = picture.has_css_class("mimick-thumbnail-square");
    if on && !has {
        picture.add_css_class("mimick-thumbnail-square");
    } else if !on && has {
        picture.remove_css_class("mimick-thumbnail-square");
    }
}

fn schedule_thumbnail_load(
    ctx: Arc<AppContext>,
    picture: gtk::Picture,
    asset_id: String,
    local_path: String,
    generation: u64,
) {
    let local_path = (!local_path.is_empty()).then(|| std::path::PathBuf::from(local_path));
    let gen_cell = generation_cell(&picture);

    glib::MainContext::default().spawn_local(async move {
        if gen_cell.get() != generation {
            return;
        }
        let cancel_cell = gen_cell.clone();
        let is_cancelled = move || cancel_cell.get() != generation;
        let cache = ctx.thumbnail_cache.clone();
        let result = match local_path {
            Some(path) => {
                cache
                    .load_local_thumbnail_cancellable(&asset_id, &path, is_cancelled.clone())
                    .await
            }
            None => {
                cache
                    .load_thumbnail_cancellable(
                        &asset_id,
                        crate::api_client::ThumbnailSize::Thumbnail,
                        is_cancelled.clone(),
                    )
                    .await
            }
        };
        if gen_cell.get() != generation {
            return;
        }
        match result {
            Ok(texture) => {
                picture.set_paintable(Some(&texture));
                set_thumb_state(&picture, ThumbState::Loaded);
            }
            Err(_) => set_thumb_state(&picture, ThumbState::Error),
        }
    });
}

const GEN_DATA_KEY: &str = "mimick-cell-gen";

fn generation_cell(picture: &gtk::Picture) -> Rc<Cell<u64>> {
    let existing = unsafe {
        picture
            .data::<Rc<Cell<u64>>>(GEN_DATA_KEY)
            .map(|p| p.as_ref().clone())
    };
    if let Some(cell) = existing {
        return cell;
    }
    let cell = Rc::new(Cell::new(0u64));
    unsafe {
        picture.set_data::<Rc<Cell<u64>>>(GEN_DATA_KEY, cell.clone());
    }
    cell
}

fn bump_generation(picture: &gtk::Picture) -> u64 {
    let cell = generation_cell(picture);
    let next = cell.get().wrapping_add(1);
    cell.set(next);
    next
}

fn decode_thumbhash_texture(thumbhash_b64: &str) -> Option<Texture> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(thumbhash_b64)
        .ok()?;
    let (w, h, rgba) = thumbhash::thumb_hash_to_rgba(&bytes).ok()?;
    let stride = w * 4;
    let pixel_bytes = glib::Bytes::from_owned(rgba);
    Some(
        gdk4::MemoryTexture::new(
            w as i32,
            h as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &pixel_bytes,
            stride,
        )
        .upcast(),
    )
}
