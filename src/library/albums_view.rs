//! Albums landing page with Recent / Owned / Shared sections.
//!
//! Fetches album cover thumbnails and renders them as clickable tiles
//! in a responsive flow layout. Selecting a tile navigates the main
//! grid to that album's contents.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;

use crate::api_client::{LibraryAlbum, ThumbnailSize};
use crate::app_context::AppContext;

pub type AlbumClick = Rc<dyn Fn(&str, String)>;

/// Sort modes available for the albums landing page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlbumsSort {
    #[default]
    Newest,
    Name,
    MostAssets,
}

/// Contains references to individual grid widgets of the albums overview display.
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
    cached_albums: Rc<RefCell<Vec<LibraryAlbum>>>,
    cached_click: Rc<RefCell<Option<AlbumClick>>>,
    pub search_query: Rc<RefCell<String>>,
    pub filter_entry: gtk::SearchEntry,
    pub sort_mode: Rc<Cell<AlbumsSort>>,
    cached_ctx: Rc<RefCell<Option<Arc<AppContext>>>>,
}

/// Construct the hierarchical panels and containers for the albums listing page.
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
    let filter_entry = gtk::SearchEntry::builder()
        .placeholder_text("Filter albums")
        .hexpand(true)
        .max_width_chars(20)
        .build();
    header_row.append(&title);
    header_row.append(&filter_entry);
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
        cached_albums: Rc::new(RefCell::new(Vec::new())),
        cached_click: Rc::new(RefCell::new(None)),
        search_query: Rc::new(RefCell::new(String::new())),
        filter_entry,
        sort_mode: Rc::new(Cell::new(AlbumsSort::default())),
        cached_ctx: Rc::new(RefCell::new(None)),
    }
}

/// Populate the album list grids grouped by ownership status.
pub fn populate_albums(
    parts: &AlbumsViewParts,
    ctx: Arc<AppContext>,
    albums: Vec<LibraryAlbum>,
    on_click: AlbumClick,
) {
    *parts.cached_albums.borrow_mut() = albums;
    *parts.cached_click.borrow_mut() = Some(on_click);
    *parts.cached_ctx.borrow_mut() = Some(ctx);
    render_albums(parts);
}

/// Set the current search filter and re-render. Empty string clears the filter.
pub fn set_search_filter(parts: &AlbumsViewParts, query: &str) {
    *parts.search_query.borrow_mut() = query.to_string();
    render_albums(parts);
}

/// Set the current sort mode and re-render.
pub fn set_sort_mode(parts: &AlbumsViewParts, mode: AlbumsSort) {
    parts.sort_mode.set(mode);
    render_albums(parts);
}

/// Pure projection over the cached album list. Returns the (recent, owned,
/// shared) buckets that drive the three Albums-view rows after applying the
/// active search filter and sort mode. Extracted from `render_albums` so the
/// ordering rules can be unit-tested without GTK.
pub(crate) fn project_albums<'a>(
    albums: &'a [LibraryAlbum],
    current_user: &str,
    query: &str,
    sort: AlbumsSort,
) -> (
    Vec<&'a LibraryAlbum>,
    Vec<&'a LibraryAlbum>,
    Vec<&'a LibraryAlbum>,
) {
    let query = query.to_ascii_lowercase();
    let filtered: Vec<&LibraryAlbum> = albums
        .iter()
        .filter(|a| query.is_empty() || a.album_name.to_ascii_lowercase().contains(&query))
        .collect();

    let mut sorted: Vec<&LibraryAlbum> = filtered.clone();
    match sort {
        AlbumsSort::Newest => sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        AlbumsSort::Name => sorted.sort_by(|a, b| {
            a.album_name
                .to_ascii_lowercase()
                .cmp(&b.album_name.to_ascii_lowercase())
        }),
        AlbumsSort::MostAssets => sorted.sort_by_key(|a| std::cmp::Reverse(a.asset_count)),
    }

    // The "Recent" bucket always reflects creation order, even when the user
    // picks a different sort for the main owned/shared rows.
    let recent: Vec<&LibraryAlbum> = if matches!(sort, AlbumsSort::Newest) {
        sorted.iter().copied().take(8).collect()
    } else {
        let mut by_date: Vec<&LibraryAlbum> = filtered.clone();
        by_date.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        by_date.into_iter().take(8).collect()
    };

    let owned: Vec<&LibraryAlbum> = sorted
        .iter()
        .copied()
        .filter(|a| !current_user.is_empty() && a.owner_id() == current_user)
        .collect();
    let shared: Vec<&LibraryAlbum> = sorted
        .iter()
        .copied()
        .filter(|a| !current_user.is_empty() && a.owner_id() != current_user)
        .collect();

    (recent, owned, shared)
}

fn render_albums(parts: &AlbumsViewParts) {
    let ctx_opt = parts.cached_ctx.borrow().clone();
    let on_click_opt = parts.cached_click.borrow().clone();
    let (Some(ctx), Some(on_click)) = (ctx_opt, on_click_opt) else {
        return;
    };

    clear(&parts.recent_grid);
    clear(&parts.owned_grid);
    clear(&parts.shared_grid);

    let current_user = ctx.current_user_id.lock().clone().unwrap_or_default();
    let query = parts.search_query.borrow().clone();
    let sort = parts.sort_mode.get();

    let albums = parts.cached_albums.borrow();
    let (recent, owned, shared) = project_albums(&albums, &current_user, &query, sort);

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

/// Build a single category grid section with a text label and a flow box layout.
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
        .max_children_per_line(20)
        .min_children_per_line(2)
        .row_spacing(12)
        .column_spacing(12)
        .homogeneous(true)
        .halign(gtk::Align::Start)
        .build();
    section.append(&label);
    section.append(&grid);
    (section, grid)
}

/// Build a clickable tile representing an individual album complete with cover art.
fn album_tile(ctx: Arc<AppContext>, album: &LibraryAlbum, on_click: AlbumClick) -> gtk::Button {
    let tile_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    // Fixed-height thumbnail container (same pattern as explore_tile):
    // the Overlay sizes itself from the spacer child (100px) so portrait
    // cover images cannot inflate the row height.
    let thumb = gtk::Overlay::builder()
        .overflow(gtk::Overflow::Hidden)
        .css_classes(vec!["mimick-explore-tile".to_string()])
        .build();
    let spacer = gtk::Box::builder()
        .css_classes(vec!["mimick-explore-spacer".to_string()])
        .build();
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Cover)
        .build();
    thumb.set_child(Some(&spacer));
    thumb.add_overlay(&picture);

    let meta_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let title_label = gtk::Label::builder()
        .label(&album.album_name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .width_chars(1)
        .max_width_chars(1)
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
    tile_box.append(&thumb);
    tile_box.append(&meta_row);

    let button = gtk::Button::builder()
        .child(&tile_box)
        .css_classes(vec!["flat".to_string()])
        .build();

    if let Some(thumb_id) = album.thumbnail_asset_id.clone() {
        spawn_thumbnail(ctx, thumb_id, picture);
    }

    let id = album.id.clone();
    let name = album.album_name.clone();
    button.connect_clicked(move |_| on_click(&id, name.clone()));
    button
}

/// Asynchronously load and set the thumbnail for an album cover art picture widget.
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

/// Remove all child widgets from the specified flow box.
fn clear(flow: &gtk::FlowBox) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api_client::{AlbumUser, AlbumUserInfo};

    fn album(id: &str, name: &str, owner: &str, count: u32, created: &str) -> LibraryAlbum {
        LibraryAlbum {
            id: id.into(),
            album_name: name.into(),
            asset_count: count,
            thumbnail_asset_id: None,
            created_at: created.into(),
            updated_at: created.into(),
            description: String::new(),
            album_users: vec![AlbumUser {
                user: AlbumUserInfo { id: owner.into() },
                role: "owner".into(),
            }],
        }
    }

    fn fixture() -> Vec<LibraryAlbum> {
        vec![
            album("a1", "Beach Trip", "me", 200, "2024-06-01T00:00:00Z"),
            album("a2", "Family", "friend", 50, "2025-01-10T00:00:00Z"),
            album("a3", "Archive 2020", "me", 1000, "2020-12-31T00:00:00Z"),
            album("a4", "beach-volleyball", "me", 12, "2023-09-15T00:00:00Z"),
        ]
    }

    #[test]
    fn project_filters_by_case_insensitive_substring() {
        let items = fixture();
        let (_, owned, _) = project_albums(&items, "me", "beach", AlbumsSort::Name);
        let names: Vec<&str> = owned.iter().map(|a| a.album_name.as_str()).collect();
        assert_eq!(names, vec!["Beach Trip", "beach-volleyball"]);
    }

    #[test]
    fn project_sort_newest_orders_by_created_desc() {
        let items = fixture();
        let (_, owned, _) = project_albums(&items, "me", "", AlbumsSort::Newest);
        let ids: Vec<&str> = owned.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["a1", "a4", "a3"]);
    }

    #[test]
    fn project_sort_name_is_case_insensitive() {
        let items = fixture();
        let (_, owned, _) = project_albums(&items, "me", "", AlbumsSort::Name);
        let names: Vec<&str> = owned.iter().map(|a| a.album_name.as_str()).collect();
        // "Archive 2020", "Beach Trip", "beach-volleyball" — lowercase-sorted.
        assert_eq!(
            names,
            vec!["Archive 2020", "Beach Trip", "beach-volleyball"]
        );
    }

    #[test]
    fn project_sort_most_assets_descends() {
        let items = fixture();
        let (_, owned, _) = project_albums(&items, "me", "", AlbumsSort::MostAssets);
        let counts: Vec<u32> = owned.iter().map(|a| a.asset_count).collect();
        assert_eq!(counts, vec![1000, 200, 12]);
    }

    #[test]
    fn project_buckets_by_owner() {
        let items = fixture();
        let (_, owned, shared) = project_albums(&items, "me", "", AlbumsSort::Newest);
        assert_eq!(owned.len(), 3);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].id, "a2");
    }

    #[test]
    fn project_empty_current_user_excludes_from_both_buckets() {
        // Without a known user id we can't safely classify owner vs shared,
        // so both buckets stay empty to avoid mis-labeling.
        let items = fixture();
        let (recent, owned, shared) = project_albums(&items, "", "", AlbumsSort::Newest);
        assert!(owned.is_empty());
        assert!(shared.is_empty());
        // Recent still populates regardless of user identity.
        assert_eq!(recent.len(), 4);
    }

    #[test]
    fn project_recent_always_by_date_even_under_alternate_sort() {
        let items = fixture();
        let (recent, _, _) = project_albums(&items, "me", "", AlbumsSort::MostAssets);
        let ids: Vec<&str> = recent.iter().map(|a| a.id.as_str()).collect();
        // Recent is creation-order, not asset-count, so the "Family" album
        // (newest) leads even though it's not the largest.
        assert_eq!(ids[0], "a2");
    }

    #[test]
    fn project_recent_caps_at_eight() {
        let mut items = Vec::new();
        for i in 0..20 {
            items.push(album(
                &format!("id{i}"),
                &format!("Album {i}"),
                "me",
                i,
                &format!("2024-01-{:02}T00:00:00Z", i + 1),
            ));
        }
        let (recent, _, _) = project_albums(&items, "me", "", AlbumsSort::Newest);
        assert_eq!(recent.len(), 8);
    }
}
