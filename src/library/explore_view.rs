//! Sectioned landing page mirroring Immich web's Explore tab.
//!
//! Three rows: People (round avatars), Places (city tiles), Things (tag
//! tiles). Tile clicks invoke caller-provided closures so dispatch lives in
//! `mod.rs` and this module stays UI-only.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gdk4::Texture;
use glib::Bytes;
use gtk::prelude::*;

use crate::api_client::{ExploreSection, Person, ThumbnailSize};
use crate::app_context::AppContext;

type ExploreClick = Rc<dyn Fn(&str, String)>;

pub struct ExploreViewParts {
    pub root: gtk::ScrolledWindow,
    pub populated: Rc<Cell<bool>>,
    people_row: gtk::Box,
    places_grid: gtk::FlowBox,
    things_grid: gtk::FlowBox,
    people_section: gtk::Box,
    places_section: gtk::Box,
    things_section: gtk::Box,
}

pub fn build_explore_view() -> ExploreViewParts {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let (people_section, people_row) = build_people_section();
    let (places_section, places_grid) = build_tile_section("Places");
    let (things_section, things_grid) = build_tile_section("Things");

    outer.append(&people_section);
    outer.append(&places_section);
    outer.append(&things_section);

    let root = gtk::ScrolledWindow::builder()
        .child(&outer)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();

    ExploreViewParts {
        root,
        populated: Rc::new(Cell::new(false)),
        people_row,
        places_grid,
        things_grid,
        people_section,
        places_section,
        things_section,
    }
}

fn build_people_section() -> (gtk::Box, gtk::Box) {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .visible(false)
        .build();
    section.append(&heading("People"));
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&row)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .height_request(140)
        .build();
    section.append(&scroller);
    (section, row)
}

fn build_tile_section(title: &str) -> (gtk::Box, gtk::FlowBox) {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .visible(false)
        .build();
    section.append(&heading(title));
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .min_children_per_line(3)
        .max_children_per_line(16)
        .homogeneous(true)
        .build();
    section.append(&grid);
    (section, grid)
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(vec!["title-2".to_string()])
        .build()
}

pub fn populate_people<F>(
    parts: &ExploreViewParts,
    ctx: Arc<AppContext>,
    people: Vec<Person>,
    on_click: F,
) where
    F: Fn(String, String) + 'static,
{
    while let Some(child) = parts.people_row.first_child() {
        parts.people_row.remove(&child);
    }
    parts.people_section.set_visible(!people.is_empty());
    let on_click = Rc::new(on_click);
    for person in people.into_iter().take(40) {
        let tile = person_tile(ctx.clone(), &person, on_click.clone());
        parts.people_row.append(&tile);
    }
}

pub fn populate_explore<F>(
    parts: &ExploreViewParts,
    ctx: Arc<AppContext>,
    sections: Vec<ExploreSection>,
    on_click: F,
) where
    F: Fn(&str, String) + 'static,
{
    while let Some(child) = parts.places_grid.first_child() {
        parts.places_grid.remove(&child);
    }
    while let Some(child) = parts.things_grid.first_child() {
        parts.things_grid.remove(&child);
    }
    let mut had_places = false;
    let mut had_things = false;

    let on_click = Rc::new(on_click);
    for section in sections {
        // Places use `exifInfo.city`; everything else (smartInfo.tags /
        // smartInfo.objects / future fields) routes into Things so we never
        // silently drop a section the server returned.
        let kind = if section.field_name.contains("city") {
            "place"
        } else {
            "thing"
        };
        let target_grid = if kind == "place" {
            had_places = true;
            &parts.places_grid
        } else {
            had_things = true;
            &parts.things_grid
        };
        for item in section.items.into_iter().take(24) {
            let tile = explore_tile(
                ctx.clone(),
                kind,
                &item.value,
                &item.data.id,
                on_click.clone(),
            );
            target_grid.append(&tile);
        }
    }

    parts.places_section.set_visible(had_places);
    parts.things_section.set_visible(had_things);
}

fn person_tile(
    ctx: Arc<AppContext>,
    person: &Person,
    on_click: Rc<dyn Fn(String, String)>,
) -> gtk::Button {
    let avatar = gtk::Picture::builder()
        .width_request(96)
        .height_request(96)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Cover)
        .css_classes(vec!["mimick-person-avatar".to_string()])
        .build();
    let label = gtk::Label::builder()
        .label(if person.name.is_empty() {
            "Unnamed"
        } else {
            &person.name
        })
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(12)
        .build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    inner.append(&avatar);
    inner.append(&label);

    let button = gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["flat".to_string()])
        .build();

    let id = person.id.clone();
    let name = if person.name.is_empty() {
        "Unnamed".to_string()
    } else {
        person.name.clone()
    };
    button.connect_clicked(move |_| on_click(id.clone(), name.clone()));

    spawn_person_thumbnail(ctx, person.id.clone(), avatar);
    button
}

fn explore_tile(
    ctx: Arc<AppContext>,
    kind: &'static str,
    value: &str,
    asset_id: &str,
    on_click: ExploreClick,
) -> gtk::Button {
    let picture = gtk::Picture::builder()
        .width_request(128)
        .height_request(128)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Cover)
        .css_classes(vec!["mimick-explore-tile".to_string()])
        .build();
    let label = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(vec!["caption-heading".to_string()])
        .build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    inner.append(&picture);
    inner.append(&label);

    let button = gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["flat".to_string()])
        .build();

    let value_owned = value.to_string();
    button.connect_clicked(move |_| on_click(kind, value_owned.clone()));

    spawn_asset_thumbnail(ctx, asset_id.to_string(), picture);
    button
}

fn spawn_asset_thumbnail(ctx: Arc<AppContext>, asset_id: String, picture: gtk::Picture) {
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

fn spawn_person_thumbnail(ctx: Arc<AppContext>, person_id: String, picture: gtk::Picture) {
    glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
        glib::MainContext::default().spawn_local(async move {
            let bytes = match ctx.api_client.fetch_person_thumbnail(&person_id).await {
                Ok(b) => b,
                Err(_) => return,
            };
            let texture = tokio::task::spawn_blocking(move || -> Option<Texture> {
                Texture::from_bytes(&Bytes::from(&bytes[..])).ok()
            })
            .await
            .ok()
            .flatten();
            if let Some(texture) = texture {
                picture.set_paintable(Some(&texture));
            }
        });
    });
}
