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
            .build();
        let picture = gtk::Picture::builder()
            .width_request(160)
            .height_request(160)
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Cover)
            .build();
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(22)
            .build();
        let status = gtk::Image::builder()
            .icon_name("cloud-symbolic")
            .halign(gtk::Align::Start)
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
        status.set_icon_name(Some(sync_icon_name(sync_state)));

        let cache = ctx.thumbnail_cache.clone();
        let picture_clone = picture.clone();

        if let Some(texture) =
            cache.get_cached(&asset_id, crate::api_client::ThumbnailSize::Thumbnail)
        {
            picture.set_paintable(Some(&texture));
            return;
        }

        glib::MainContext::default().spawn_local(async move {
            if let Ok(texture) = cache
                .load_thumbnail(&asset_id, crate::api_client::ThumbnailSize::Thumbnail)
                .await
                && picture_clone.tooltip_text().as_deref() == Some(asset_id.as_str())
            {
                picture_clone.set_paintable(Some(&texture));
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
            picture.set_tooltip_text(None);
            picture.set_paintable(Option::<&Texture>::None);
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

fn sync_icon_name(sync_state: u32) -> &'static str {
    match sync_state {
        2 => "folder-cloud-symbolic",
        1 => "folder-symbolic",
        _ => "cloud-symbolic",
    }
}
