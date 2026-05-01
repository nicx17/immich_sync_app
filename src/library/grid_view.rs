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
        let container = gtk::Overlay::builder()
            .css_classes(vec!["mimick-cell".to_string()])
            .build();
        // `Cover` keeps the image's aspect ratio but fills the cell by cropping
        // overflow — no transparent letterbox gaps between thumbnails. Width
        // is driven by GridView column allocation so cells expand edge-to-edge.
        let picture = gtk::Picture::builder()
            .height_request(200)
            .can_shrink(true)
            .hexpand(true)
            .content_fit(gtk::ContentFit::Cover)
            .css_classes(vec!["mimick-thumbnail-loading".to_string()])
            .build();

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
        container.add_overlay(&status);
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
        let Some(container) = list_item.child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let Some(picture) = container.child().and_downcast::<gtk::Picture>() else {
            return;
        };
        let Some(status) = container.last_child().and_downcast::<gtk::Image>() else {
            return;
        };

        let asset_id = item.property::<String>("id");
        let local_path = item.property::<String>("local-path");
        let sync_state = item.property::<u32>("sync-state");
        picture.set_tooltip_text(Some(&asset_id));
        picture.set_paintable(Option::<&Texture>::None);
        set_thumb_state(&picture, ThumbState::Loading);
        status.set_icon_name(Some(sync_icon_name(sync_state)));
        status.set_tooltip_text(Some(sync_state_label(sync_state)));

        // Timeline source: hide the per-cell sync badge and switch the
        // thumbnail to square corners so the visual matches the Immich
        // web app's flat grid. The check happens here instead of at grid
        // build time because cells are reused across source switches —
        // when the user toggles Timeline on/off, already-realised cells
        // need to re-render in the new style.
        let in_timeline = matches!(
            ctx.library_state.lock().unwrap().source,
            crate::library::state::LibrarySource::Timeline,
        );
        status.set_visible(!in_timeline);
        if in_timeline {
            picture.add_css_class("mimick-thumbnail-square");
        } else {
            picture.remove_css_class("mimick-thumbnail-square");
        }

        let cache = ctx.thumbnail_cache.clone();
        let picture_clone = picture.clone();

        // Memory-cache fast path: works for both local and remote rows since
        // `load_local_thumbnail` and `load_thumbnail` share the same key shape.
        if let Some(texture) =
            cache.get_cached(&asset_id, crate::api_client::ThumbnailSize::Thumbnail)
        {
            picture.set_paintable(Some(&texture));
            set_thumb_state(&picture, ThumbState::Loaded);
            return;
        }

        if !local_path.is_empty() {
            let path = std::path::PathBuf::from(&local_path);
            let asset_id_clone = asset_id.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = cache.load_local_thumbnail(&asset_id_clone, &path).await;
                if picture_clone.tooltip_text().as_deref() != Some(asset_id_clone.as_str()) {
                    return;
                }
                match result {
                    Ok(texture) => {
                        picture_clone.set_paintable(Some(&texture));
                        set_thumb_state(&picture_clone, ThumbState::Loaded);
                    }
                    Err(_) => set_thumb_state(&picture_clone, ThumbState::Error),
                }
            });
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
        if let Some(container) = list_item.child().and_downcast::<gtk::Overlay>()
            && let Some(picture) = container.child().and_downcast::<gtk::Picture>()
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
    }
}

pub fn replace_model(model: &gtk::gio::ListStore, objects: &[AssetObject]) {
    model.remove_all();
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
