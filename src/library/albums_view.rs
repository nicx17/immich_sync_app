//! Albums landing page with Recent / Owned / Shared sections.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;

use crate::api_client::{LibraryAlbum, ThumbnailSize};
use crate::app_context::AppContext;

pub type AlbumClick = Rc<dyn Fn(&str, String)>;

pub struct AlbumsViewParts {
    pub root: gtk::ScrolledWindow,
    pub populated: Rc<Cell<bool>>,
    pub create_button: gtk::Button,
    recent_grid: gtk::FlowBox,
    owned_grid: gtk::FlowBox,
    shared_grid: gtk::FlowBox,
    recent_section: gtk::Box,
    owned_section: gtk::Box,
    shared_section: gtk::Box,
}

pub fn build_albums_view() -> AlbumsViewParts {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let header_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let title = gtk::Label::builder()
        .label("Albums")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["title-1".to_string()])
        .build();
    let create_button = gtk::Button::builder()
        .label("Create album")
        .css_classes(vec!["suggested-action".to_string()])
        .build();
    header_row.append(&title);
    header_row.append(&create_button);
    outer.append(&header_row);

    let (recent_section, recent_grid) = build_section("Recent");
    let (owned_section, owned_grid) = build_section("Your albums");
    let (shared_section, shared_grid) = build_section("Shared with you");
    outer.append(&recent_section);
    outer.append(&owned_section);
    outer.append(&shared_section);

    let root = gtk::ScrolledWindow::builder()
        .child(&outer)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    AlbumsViewParts {
        root,
        populated: Rc::new(Cell::new(false)),
        create_button,
        recent_grid,
        owned_grid,
        shared_grid,
        recent_section,
        owned_section,
        shared_section,
    }
}

pub fn populate_albums(
    parts: &AlbumsViewParts,
    ctx: Arc<AppContext>,
    albums: Vec<LibraryAlbum>,
    on_click: AlbumClick,
) {
    clear(&parts.recent_grid);
    clear(&parts.owned_grid);
    clear(&parts.shared_grid);

    let current_user = ctx
        .current_user_id
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_default();

    let mut recent: Vec<&LibraryAlbum> = albums.iter().collect();
    recent.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let recent: Vec<&LibraryAlbum> = recent.into_iter().take(8).collect();

    let owned: Vec<&LibraryAlbum> = albums
        .iter()
        .filter(|a| !current_user.is_empty() && a.owner_id == current_user)
        .collect();
    let shared: Vec<&LibraryAlbum> = albums
        .iter()
        .filter(|a| !current_user.is_empty() && a.owner_id != current_user)
        .collect();

    for album in &recent {
        parts
            .recent_grid
            .insert(&album_tile(ctx.clone(), album, on_click.clone()), -1);
    }
    for album in &owned {
        parts
            .owned_grid
            .insert(&album_tile(ctx.clone(), album, on_click.clone()), -1);
    }
    for album in &shared {
        parts
            .shared_grid
            .insert(&album_tile(ctx.clone(), album, on_click.clone()), -1);
    }

    parts.recent_section.set_visible(!recent.is_empty());
    parts.owned_section.set_visible(!owned.is_empty());
    parts.shared_section.set_visible(!shared.is_empty());
    parts.populated.set(true);
}

fn build_section(title: &str) -> (gtk::Box, gtk::FlowBox) {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(vec!["title-3".to_string()])
        .build();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(12)
        .min_children_per_line(1)
        .row_spacing(12)
        .column_spacing(12)
        .homogeneous(false)
        .halign(gtk::Align::Start)
        .build();
    section.append(&label);
    section.append(&grid);
    (section, grid)
}

fn album_tile(ctx: Arc<AppContext>, album: &LibraryAlbum, on_click: AlbumClick) -> gtk::Button {
    let tile_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    let picture = gtk::Picture::builder()
        .width_request(300)
        .height_request(220)
        .content_fit(gtk::ContentFit::Cover)
        .css_classes(vec!["mimick-explore-tile".to_string()])
        .build();
    let meta_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let title_label = gtk::Label::builder()
        .label(&album.album_name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(vec!["caption-heading".to_string()])
        .build();
    let count_label = gtk::Label::builder()
        .label(format!(
            "{} item{}",
            album.asset_count,
            if album.asset_count == 1 { "" } else { "s" }
        ))
        .xalign(1.0)
        .css_classes(vec!["caption".to_string(), "dim-label".to_string()])
        .build();
    meta_row.append(&title_label);
    meta_row.append(&count_label);
    tile_box.append(&picture);
    tile_box.append(&meta_row);

    let button = gtk::Button::builder()
        .child(&tile_box)
        .css_classes(vec!["flat".to_string()])
        .width_request(300)
        .hexpand(false)
        .halign(gtk::Align::Start)
        .build();

    if let Some(thumb_id) = album.thumbnail_asset_id.clone() {
        spawn_thumbnail(ctx, thumb_id, picture);
    }

    let id = album.id.clone();
    let name = album.album_name.clone();
    button.connect_clicked(move |_| on_click(&id, name.clone()));
    button
}

fn spawn_thumbnail(ctx: Arc<AppContext>, asset_id: String, picture: gtk::Picture) {
    if let Some(texture) = ctx
        .thumbnail_cache
        .get_cached(&asset_id, ThumbnailSize::Thumbnail)
    {
        picture.set_paintable(Some(&texture));
        return;
    }
    glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
        glib::MainContext::default().spawn_local(async move {
            if let Ok(texture) = ctx
                .thumbnail_cache
                .load_thumbnail(&asset_id, ThumbnailSize::Thumbnail)
                .await
            {
                picture.set_paintable(Some(&texture));
            }
        });
    });
}

fn clear(flow: &gtk::FlowBox) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
}
