//! GridView-based asset browser with async thumbnail loading and pagination.

use std::sync::Arc;

use gdk4::Texture;
use gtk::prelude::*;

use crate::app_context::AppContext;
use crate::library::asset_object::AssetObject;

pub struct GridViewParts {
    pub model: gtk::gio::ListStore,
    pub scrolled: gtk::ScrolledWindow,
    pub view: gtk::GridView,
}

pub fn build_grid_view(ctx: Arc<AppContext>) -> GridViewParts {
    let model = gtk::gio::ListStore::new::<AssetObject>();
    let selection = gtk::NoSelection::new(Some(model.clone()));
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let container = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .width_request(180)
            .css_classes(vec!["mimick-cell".to_string()])
            .build();
        let picture = gtk::Picture::builder()
            .width_request(160)
            .height_request(160)
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Cover)
            .css_classes(vec!["mimick-thumbnail-loading".to_string()])
            .build();
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(22)
            .css_classes(vec!["mimick-cell-name".to_string()])
            .build();
        // `pixel_size` is set explicitly so the icon renders at the badge
        // size we expect (icons inside small Boxes can render at 0px when
        // both width-request and pixel-size are unset).
        let status = gtk::Image::builder()
            .icon_name("network-server-symbolic")
            .halign(gtk::Align::Start)
            .pixel_size(14)
            .css_classes(vec!["mimick-status-badge".to_string()])
            .build();

        container.append(&picture);
        container.append(&name);
        container.append(&status);
        list_item.set_child(Some(&container));
    });

    factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let item = match list_item.item().and_downcast::<AssetObject>() {
            Some(item) => item,
            None => return,
        };
        let Some(container) = list_item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(picture) = container.first_child().and_downcast::<gtk::Picture>() else {
            return;
        };
        let Some(name) = picture.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(status) = name.next_sibling().and_downcast::<gtk::Image>() else {
            return;
        };

        let asset_id = item.property::<String>("id");
        let filename = item.property::<String>("filename");
        let sync_state = item.property::<u32>("sync-state");
        name.set_label(&filename);
        picture.set_tooltip_text(Some(&asset_id));
        picture.set_paintable(Option::<&Texture>::None);
        set_thumb_state(&picture, ThumbState::Loading);
        status.set_icon_name(Some(sync_icon_name(sync_state)));
        status.set_tooltip_text(Some(sync_state_label(sync_state)));

        let cache = ctx.thumbnail_cache.clone();
        let picture_clone = picture.clone();

        if let Some(texture) =
            cache.get_cached(&asset_id, crate::api_client::ThumbnailSize::Thumbnail)
        {
            picture.set_paintable(Some(&texture));
            set_thumb_state(&picture, ThumbState::Loaded);
            return;
        }

        glib::MainContext::default().spawn_local(async move {
            let result = cache
                .load_thumbnail(&asset_id, crate::api_client::ThumbnailSize::Thumbnail)
                .await;

            // Cell may have been rebound to a different asset while we were loading.
            if picture_clone.tooltip_text().as_deref() != Some(asset_id.as_str()) {
                return;
            }

            match result {
                Ok(texture) => {
                    picture_clone.set_paintable(Some(&texture));
                    set_thumb_state(&picture_clone, ThumbState::Loaded);
                }
                Err(_) => {
                    set_thumb_state(&picture_clone, ThumbState::Error);
                }
            }
        });
    });

    factory.connect_unbind(|_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let Some(container) = list_item.child().and_downcast::<gtk::Box>()
            && let Some(picture) = container.first_child().and_downcast::<gtk::Picture>()
        {
            // Clearing the tooltip causes any in-flight thumbnail task to discard its
            // result on completion, so a recycled cell does not flash a stale image.
            picture.set_tooltip_text(None);
            picture.set_paintable(Option::<&Texture>::None);
            set_thumb_state(&picture, ThumbState::Loading);
        }
    });

    let view = gtk::GridView::builder()
        .model(&selection)
        .factory(&factory)
        .single_click_activate(true)
        .enable_rubberband(false)
        .max_columns(6)
        .min_columns(2)
        .build();

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
    }
}

pub fn replace_model(model: &gtk::gio::ListStore, objects: &[AssetObject]) {
    model.remove_all();
    for object in objects {
        model.append(object);
    }
}

/// Map the `sync_state` enum (0 = remote-only, 1 = local-only, 2 = both) to
/// an icon name that is reliably present in `adwaita-icon-theme`. The names
/// previously used here (`cloud-symbolic`, `folder-cloud-symbolic`) ship in
/// some distro icon themes but not in the upstream Adwaita set, which is
/// why the badge rendered blank on most systems.
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
    for cls in [
        "mimick-thumbnail-loading",
        "mimick-thumbnail-loaded",
        "mimick-thumbnail-error",
    ] {
        picture.remove_css_class(cls);
    }
    let cls = match state {
        ThumbState::Loading => "mimick-thumbnail-loading",
        ThumbState::Loaded => "mimick-thumbnail-loaded",
        ThumbState::Error => "mimick-thumbnail-error",
    };
    picture.add_css_class(cls);
    if matches!(state, ThumbState::Error) {
        picture.set_tooltip_text(Some("Thumbnail unavailable"));
    }
}
