//! Sectioned landing page mirroring Immich web's Explore tab.
//!
//! Four rows: People (round avatars), Recently Added (date tiles),
//! Places (city tiles), Things (tag tiles). Tile clicks invoke
//! caller-provided closures so dispatch lives in `mod.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gdk4::Texture;
use glib::Bytes;
use gtk::prelude::*;

use crate::api_client::{ExploreSection, Person, PlaceItem, ThumbnailSize};
use crate::app_context::AppContext;

type ExploreClick = Rc<dyn Fn(&str, String, String)>;
type PersonClick = Rc<dyn Fn(String, String)>;

const INITIAL_TILE_COUNT: usize = 16;
const RECENTS_EXPANDED_COUNT: usize = 30;

/// Contains references to individual grid widgets of the explore tab dashboard display.
pub struct ExploreViewParts {
    pub root: gtk::ScrolledWindow,
    pub populated: Rc<Cell<bool>>,
    people_row: gtk::Box,
    recents_grid: gtk::FlowBox,
    places_grid: gtk::FlowBox,
    things_grid: gtk::FlowBox,
    people_section: gtk::Box,
    recents_section: gtk::Box,
    places_section: gtk::Box,
    things_section: gtk::Box,
    people_spinner: gtk::Spinner,
    recents_spinner: gtk::Spinner,
    places_spinner: gtk::Spinner,
    things_spinner: gtk::Spinner,
    pub people_filter_button: gtk::MenuButton,
    cached_people: Rc<RefCell<Vec<Person>>>,
    cached_people_click: Rc<RefCell<Option<PersonClick>>>,
    cached_places: Rc<RefCell<Vec<PlaceItem>>>,
    cached_places_click: Rc<RefCell<Option<ExploreClick>>>,
    pub search_query: Rc<RefCell<String>>,
    pub filter_entry: gtk::SearchEntry,
    cached_ctx: Rc<RefCell<Option<Arc<AppContext>>>>,
}

/// Construct the hierarchical panels and containers for the explore dashboard view.
pub fn build_explore_view() -> ExploreViewParts {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let (people_section, people_row, people_spinner, people_filter_button) = build_people_section();
    let (recents_section, recents_grid, recents_spinner) = build_tile_section("Recently Added");
    let (places_section, places_grid, places_spinner) = build_tile_section("Places");
    let (things_section, things_grid, things_spinner) = build_tile_section("Things");

    let filter_entry = gtk::SearchEntry::builder()
        .placeholder_text("Filter people and places")
        .hexpand(true)
        .max_width_chars(20)
        .build();
    outer.append(&filter_entry);

    outer.append(&people_section);
    outer.append(&places_section);
    outer.append(&recents_section);
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
        recents_grid,
        places_grid,
        things_grid,
        people_section,
        recents_section,
        places_section,
        things_section,
        people_spinner,
        recents_spinner,
        places_spinner,
        things_spinner,
        people_filter_button,
        cached_people: Rc::new(RefCell::new(Vec::new())),
        cached_people_click: Rc::new(RefCell::new(None)),
        cached_places: Rc::new(RefCell::new(Vec::new())),
        cached_places_click: Rc::new(RefCell::new(None)),
        search_query: Rc::new(RefCell::new(String::new())),
        filter_entry,
        cached_ctx: Rc::new(RefCell::new(None)),
    }
}

/// Reveal each section with its spinner active, so the user gets immediate
/// visual feedback that data is on the way. Each `populate_*` call clears
/// its own spinner when results arrive.
pub fn show_loading(parts: &ExploreViewParts) {
    for (section, spinner) in [
        (&parts.people_section, &parts.people_spinner),
        (&parts.recents_section, &parts.recents_spinner),
        (&parts.places_section, &parts.places_spinner),
        (&parts.things_section, &parts.things_spinner),
    ] {
        section.set_visible(true);
        spinner.set_visible(true);
        spinner.start();
    }
}

fn stop_spinner(spinner: &gtk::Spinner) {
    spinner.stop();
    spinner.set_visible(false);
}

/// Build a horizontal scrolled gallery row dedicated to recognized people circles.
fn build_people_section() -> (gtk::Box, gtk::Box, gtk::Spinner, gtk::MenuButton) {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .visible(false)
        .build();

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let title = heading("People");
    title.set_hexpand(true);
    header.append(&title);
    let spinner = gtk::Spinner::builder()
        .visible(false)
        .valign(gtk::Align::Center)
        .build();
    header.append(&spinner);
    let filter_button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("Filter people")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build();
    header.append(&filter_button);
    section.append(&header);

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
    (section, row, spinner, filter_button)
}

/// Build a flow grid section mapping image tiles for Places or Things category.
fn build_tile_section(title: &str) -> (gtk::Box, gtk::FlowBox, gtk::Spinner) {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .visible(false)
        .build();
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let title_label = heading(title);
    title_label.set_hexpand(true);
    header.append(&title_label);
    let spinner = gtk::Spinner::builder()
        .visible(false)
        .valign(gtk::Align::Center)
        .build();
    header.append(&spinner);
    section.append(&header);
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .min_children_per_line(2)
        .max_children_per_line(20)
        .homogeneous(true)
        .halign(gtk::Align::Start)
        .build();
    section.append(&grid);
    (section, grid, spinner)
}

/// Helper to create a styled section heading label.
fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(vec!["title-2".to_string()])
        .build()
}

/// Populate the round avatar buttons for recognized people in the dashboard section.
///
/// Stores the full fetched list so face-visibility toggles can re-filter without
/// re-querying the server. Always call with `include_hidden=true` upstream so the
/// `Show hidden` toggle can apply locally.
pub fn populate_people<F>(
    parts: &ExploreViewParts,
    ctx: Arc<AppContext>,
    people: Vec<Person>,
    on_click: F,
) where
    F: Fn(String, String) + 'static,
{
    stop_spinner(&parts.people_spinner);
    *parts.cached_people.borrow_mut() = people;
    *parts.cached_people_click.borrow_mut() = Some(Rc::new(on_click));
    *parts.cached_ctx.borrow_mut() = Some(ctx.clone());
    render_people(parts, ctx);
}

/// Apply (or clear) the search filter on the people row. Pass an empty string
/// to disable filtering. Caller drives this from the header-bar search entry
/// when the Explore view is the active content stack child.
pub fn set_people_search(parts: &ExploreViewParts, query: &str) {
    *parts.search_query.borrow_mut() = query.to_string();
    let Some(ctx) = parts.cached_ctx.borrow().clone() else {
        return;
    };
    render_people(parts, ctx.clone());
    render_places_filtered(parts, ctx);
}

/// Re-render the places section applying the current search query.
fn render_places_filtered(parts: &ExploreViewParts, ctx: Arc<AppContext>) {
    while let Some(child) = parts.places_grid.first_child() {
        parts.places_grid.remove(&child);
    }
    let places = parts.cached_places.borrow();
    let query = parts.search_query.borrow().to_ascii_lowercase();
    let filtered: Vec<&PlaceItem> = if query.is_empty() {
        places.iter().collect()
    } else {
        places
            .iter()
            .filter(|p| p.city.to_ascii_lowercase().contains(&query))
            .collect()
    };
    parts.places_section.set_visible(!filtered.is_empty());
    let on_click = match parts.cached_places_click.borrow().clone() {
        Some(cb) => cb,
        None => return,
    };
    for place in filtered.into_iter().take(INITIAL_TILE_COUNT) {
        let tile = explore_tile(
            ctx.clone(),
            "place",
            &place.city,
            &place.asset_id,
            on_click.clone(),
        );
        parts.places_grid.append(&tile);
    }
}

fn render_people(parts: &ExploreViewParts, ctx: Arc<AppContext>) {
    while let Some(child) = parts.people_row.first_child() {
        parts.people_row.remove(&child);
    }
    let (show_unnamed, show_hidden) = {
        let cfg = ctx.config.read();
        (cfg.data.show_unnamed_faces, cfg.data.show_hidden_faces)
    };
    let cached = parts.cached_people.borrow();
    let query = parts.search_query.borrow().to_ascii_lowercase();
    let filtered: Vec<&Person> = cached
        .iter()
        .filter(|p| show_hidden || !p.is_hidden)
        .filter(|p| show_unnamed || !p.name.is_empty())
        .filter(|p| query.is_empty() || p.name.to_ascii_lowercase().contains(&query))
        .collect();
    parts.people_section.set_visible(!filtered.is_empty());
    let on_click = parts.cached_people_click.borrow().clone();
    let Some(on_click) = on_click else {
        return;
    };
    for person in filtered.into_iter().take(40) {
        let tile = person_tile(ctx.clone(), person, on_click.clone());
        parts.people_row.append(&tile);
    }
}

/// Wire the filter MenuButton popover. Called once after the explore view is built.
/// `on_change` is invoked when a toggle flips so the caller can re-fetch with a
/// different `include_hidden` flag if needed.
pub fn wire_people_filter<F>(parts: &ExploreViewParts, ctx: Arc<AppContext>, on_change: F)
where
    F: Fn() + 'static,
{
    let popover = gtk::Popover::builder().build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();

    let (show_unnamed, show_hidden) = {
        let cfg = ctx.config.read();
        (cfg.data.show_unnamed_faces, cfg.data.show_hidden_faces)
    };

    let unnamed_check = gtk::CheckButton::builder()
        .label("Show unnamed")
        .active(show_unnamed)
        .build();
    let hidden_check = gtk::CheckButton::builder()
        .label("Show hidden")
        .active(show_hidden)
        .build();
    body.append(&unnamed_check);
    body.append(&hidden_check);
    popover.set_child(Some(&body));
    parts.people_filter_button.set_popover(Some(&popover));

    let on_change = Rc::new(on_change);

    let ctx_a = ctx.clone();
    let parts_a = clone_parts_handles(parts);
    let on_change_a = on_change.clone();
    unnamed_check.connect_toggled(move |btn| {
        {
            let mut cfg = ctx_a.config.write();
            cfg.data.show_unnamed_faces = btn.is_active();
            if !cfg.save() {
                log::error!("Failed to save config after toggling show_unnamed_faces");
            }
        }
        render_people(&parts_a, ctx_a.clone());
        on_change_a();
    });

    let ctx_b = ctx.clone();
    let parts_b = clone_parts_handles(parts);
    let on_change_b = on_change.clone();
    hidden_check.connect_toggled(move |btn| {
        let active = btn.is_active();
        let hidden_count = parts_b
            .cached_people
            .borrow()
            .iter()
            .filter(|p| p.is_hidden)
            .count();
        log::debug!(
            "show_hidden_faces toggled to {} ({} hidden people in cache)",
            active,
            hidden_count
        );
        {
            let mut cfg = ctx_b.config.write();
            cfg.data.show_hidden_faces = active;
            if !cfg.save() {
                log::error!("Failed to save config after toggling show_hidden_faces");
            }
        }
        render_people(&parts_b, ctx_b.clone());
        on_change_b();
    });
}

/// Build a lightweight `ExploreViewParts` snapshot containing only the widget/handle
/// references render_people needs, sharing the same `Rc` data with the original.
fn clone_parts_handles(parts: &ExploreViewParts) -> ExploreViewParts {
    ExploreViewParts {
        root: parts.root.clone(),
        populated: parts.populated.clone(),
        people_row: parts.people_row.clone(),
        recents_grid: parts.recents_grid.clone(),
        places_grid: parts.places_grid.clone(),
        things_grid: parts.things_grid.clone(),
        people_section: parts.people_section.clone(),
        recents_section: parts.recents_section.clone(),
        places_section: parts.places_section.clone(),
        things_section: parts.things_section.clone(),
        people_spinner: parts.people_spinner.clone(),
        recents_spinner: parts.recents_spinner.clone(),
        places_spinner: parts.places_spinner.clone(),
        things_spinner: parts.things_spinner.clone(),
        people_filter_button: parts.people_filter_button.clone(),
        cached_people: parts.cached_people.clone(),
        cached_people_click: parts.cached_people_click.clone(),
        cached_places: parts.cached_places.clone(),
        cached_places_click: parts.cached_places_click.clone(),
        search_query: parts.search_query.clone(),
        filter_entry: parts.filter_entry.clone(),
        cached_ctx: parts.cached_ctx.clone(),
    }
}

/// Populate the city tiles representing locations in the places dashboard section.
///
/// Caches places data so subsequent visits don't re-fetch from the server.
/// Shows the first 16 tiles with a "See More" button to expand.
pub fn populate_places<F>(
    parts: &ExploreViewParts,
    ctx: Arc<AppContext>,
    places: Vec<PlaceItem>,
    on_click: F,
) where
    F: Fn(&str, String, String) + 'static,
{
    stop_spinner(&parts.places_spinner);
    *parts.cached_places.borrow_mut() = places;
    *parts.cached_places_click.borrow_mut() = Some(Rc::new(on_click));
    render_places(parts, ctx, false);
}

/// Check if places are already cached and render them without fetching.
pub fn has_cached_places(parts: &ExploreViewParts) -> bool {
    !parts.cached_places.borrow().is_empty()
}

/// Re-render places from cache (used when navigating back to explore).
pub fn render_cached_places(parts: &ExploreViewParts, ctx: Arc<AppContext>) {
    stop_spinner(&parts.places_spinner);
    render_places(parts, ctx, false);
}

fn render_places(parts: &ExploreViewParts, ctx: Arc<AppContext>, expanded: bool) {
    while let Some(child) = parts.places_grid.first_child() {
        parts.places_grid.remove(&child);
    }
    let places = parts.cached_places.borrow();
    parts.places_section.set_visible(!places.is_empty());
    let on_click = match parts.cached_places_click.borrow().clone() {
        Some(cb) => cb,
        None => return,
    };
    let limit = if expanded {
        places.len()
    } else {
        INITIAL_TILE_COUNT
    };
    for place in places.iter().take(limit) {
        let tile = explore_tile(
            ctx.clone(),
            "place",
            &place.city,
            &place.asset_id,
            on_click.clone(),
        );
        parts.places_grid.append(&tile);
    }
    if !expanded && places.len() > INITIAL_TILE_COUNT {
        let remaining = places.len() - INITIAL_TILE_COUNT;
        drop(places);
        append_see_more_button(&parts.places_grid, remaining, {
            let parts = clone_parts_handles(parts);
            let ctx = ctx.clone();
            move || render_places(&parts, ctx.clone(), true)
        });
    } else if expanded && places.len() > INITIAL_TILE_COUNT {
        drop(places);
        append_show_less_button(&parts.places_grid, {
            let parts = clone_parts_handles(parts);
            let ctx = ctx.clone();
            move || render_places(&parts, ctx.clone(), false)
        });
    }
}

/// Populate explore sections: Things tiles + Recently Added tiles.
///
/// Sections by `field_name`:
///   - `exifInfo.city`          -- skipped (populated by `populate_places`)
///   - `createdAt`              -- rendered as "Recently Added" tiles
///   - `smartInfo.objects/tags` -- rendered as "Things" tiles
pub fn populate_explore<F>(
    parts: &ExploreViewParts,
    ctx: Arc<AppContext>,
    sections: Vec<ExploreSection>,
    on_click: F,
) where
    F: Fn(&str, String, String) + 'static,
{
    stop_spinner(&parts.things_spinner);
    stop_spinner(&parts.recents_spinner);
    while let Some(child) = parts.things_grid.first_child() {
        parts.things_grid.remove(&child);
    }
    while let Some(child) = parts.recents_grid.first_child() {
        parts.recents_grid.remove(&child);
    }
    let mut had_things = false;
    let mut had_recents = false;

    log::debug!(
        "Explore sections: [{}]",
        sections
            .iter()
            .map(|s| format!("{}({})", s.field_name, s.items.len()))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let on_click: ExploreClick = Rc::new(on_click);
    for section in sections {
        if section.field_name.contains("city") {
            continue;
        }

        // Immich v3 recently-added section.
        if section.field_name == "createdAt" || section.field_name == "updatedAt" {
            had_recents = true;
            let mut items: Vec<_> = section.items.into_iter().collect();
            items.sort_by(|a, b| b.value.cmp(&a.value));
            render_recents_tiles(parts, &ctx, &items, &on_click, false);
            if items.len() > INITIAL_TILE_COUNT {
                let remaining = items.len().min(RECENTS_EXPANDED_COUNT) - INITIAL_TILE_COUNT;
                let parts_clone = clone_parts_handles(parts);
                let ctx_clone = ctx.clone();
                let on_click_clone = on_click.clone();
                append_see_more_button(&parts.recents_grid, remaining, move || {
                    render_recents_tiles(&parts_clone, &ctx_clone, &items, &on_click_clone, true);
                });
            }
            continue;
        }

        // Only render smartInfo sections as Things tiles.
        if !section.field_name.starts_with("smartInfo") {
            log::debug!("Skipping unknown section '{}'", section.field_name);
            continue;
        }

        had_things = true;
        for item in section.items.into_iter().take(24) {
            let tile = explore_tile(
                ctx.clone(),
                "thing",
                &item.value,
                &item.data.id,
                on_click.clone(),
            );
            parts.things_grid.append(&tile);
        }
    }

    parts.things_section.set_visible(had_things);
    parts.recents_section.set_visible(had_recents);
}

/// Render recently-added tiles into the recents grid.
fn render_recents_tiles(
    parts: &ExploreViewParts,
    ctx: &Arc<AppContext>,
    items: &[crate::api_client::ExploreItem],
    on_click: &ExploreClick,
    expanded: bool,
) {
    while let Some(child) = parts.recents_grid.first_child() {
        parts.recents_grid.remove(&child);
    }
    let limit = if expanded {
        RECENTS_EXPANDED_COUNT
    } else {
        INITIAL_TILE_COUNT
    };
    for item in items.iter().take(limit) {
        let label = format_relative_date(&item.value);
        let tile = explore_tile(
            ctx.clone(),
            "recent",
            &label,
            &item.data.id,
            on_click.clone(),
        );
        parts.recents_grid.append(&tile);
    }
}

fn append_action_button<F: Fn() + 'static>(
    grid: &gtk::FlowBox,
    icon_name: &str,
    label_text: &str,
    on_click: F,
) {
    let icon = gtk::Image::builder()
        .icon_name(icon_name)
        .pixel_size(24)
        .halign(gtk::Align::Center)
        .build();
    let label = gtk::Label::builder()
        .label(label_text)
        .css_classes(["caption-heading"])
        .halign(gtk::Align::Center)
        .build();
    // Spacer forces the same min-height as explore tiles.
    let spacer = gtk::Box::builder()
        .css_classes(["mimick-explore-spacer"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    content.append(&icon);
    content.append(&label);
    // Overlay the centered label on top of the spacer so the button
    // has the same footprint as a regular tile.
    let overlay = gtk::Overlay::builder()
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["mimick-see-more-tile"])
        .build();
    overlay.set_child(Some(&spacer));
    overlay.add_overlay(&content);

    let btn = gtk::Button::builder()
        .child(&overlay)
        .css_classes(["flat"])
        .build();
    let grid_ref = grid.clone();
    btn.connect_clicked(move |button| {
        if let Some(parent) = button.parent() {
            grid_ref.remove(&parent);
        }
        on_click();
    });
    grid.append(&btn);
}

/// Append a card-sized "See More" tile to a FlowBox grid.
///
/// Matches the dimensions of adjacent explore tiles so the button fills a
/// full card slot rather than appearing as a small inline text link.
fn append_see_more_button<F: Fn() + 'static>(grid: &gtk::FlowBox, remaining: usize, on_expand: F) {
    append_action_button(
        grid,
        "view-more-symbolic",
        &format!("See {remaining} more"),
        on_expand,
    );
}

/// Append a card-sized "Show Less" tile to collapse an expanded section.
fn append_show_less_button<F: Fn() + 'static>(grid: &gtk::FlowBox, on_collapse: F) {
    append_action_button(grid, "go-up-symbolic", "Show Less", on_collapse);
}
fn parse_iso_date(iso: &str) -> Option<(i64, u32, u32, u32, u32, u32)> {
    let parsed = iso
        .replace('T', " ")
        .replace('Z', "")
        .chars()
        .take(19)
        .collect::<String>();

    let parts: Vec<&str> = parsed.split(&['-', ' ', ':'][..]).collect();
    if parts.len() < 6 {
        return None;
    }
    match (
        parts[0].parse::<i64>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
        parts[3].parse::<u32>(),
        parts[4].parse::<u32>(),
        parts[5].parse::<u32>(),
    ) {
        (Ok(y), Ok(mo), Ok(d), Ok(h), Ok(mi), Ok(s)) => Some((y, mo, d, h, mi, s)),
        _ => None,
    }
}

fn compute_epoch_seconds(year: i64, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> i64 {
    let days_in_month = [0u32, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut total_days: i64 = 0;
    for y in 1970..year {
        total_days += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
    }
    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    for m in 1..month {
        total_days += days_in_month[m as usize] as i64;
        if m == 2 && is_leap {
            total_days += 1;
        }
    }
    total_days += (day - 1) as i64;
    total_days * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64
}

/// Format an ISO 8601 timestamp into a human-readable relative label.
fn format_relative_date(iso: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let Some((year, month, day, hour, min, sec)) = parse_iso_date(iso) else {
        return iso.chars().take(10).collect();
    };

    let ts = compute_epoch_seconds(year, month, day, hour, min, sec);
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now_secs - ts;

    if diff < 60 {
        "Just now".to_string()
    } else if diff < 3600 {
        let m = diff / 60;
        format!("{m} min ago")
    } else if diff < 86400 {
        let h = diff / 3600;
        format!("{h}h ago")
    } else if diff < 86400 * 7 {
        let d = diff / 86400;
        format!("{d}d ago")
    } else {
        format!("{year:04}-{month:02}-{day:02}")
    }
}

/// Construct an individual circular avatar widget representing a recognized person face.
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

/// Construct a rectangular tile representation for a specific explore category node.
fn explore_tile(
    ctx: Arc<AppContext>,
    kind: &'static str,
    value: &str,
    asset_id: &str,
    on_click: ExploreClick,
) -> gtk::Button {
    // Fixed-height thumbnail container: the Overlay sizes itself from the
    // spacer child (100px) so portrait thumbnails cannot inflate the row
    // height.  The Picture overlay fills that space with ContentFit::Cover.
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

    let label = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .width_chars(1)
        .max_width_chars(1)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(vec!["caption-heading".to_string()])
        .build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    inner.append(&thumb);
    inner.append(&label);

    let button = gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["flat".to_string()])
        .build();

    let value_owned = value.to_string();
    let asset_id_owned = asset_id.to_string();
    button.connect_clicked(move |_| on_click(kind, value_owned.clone(), asset_id_owned.clone()));

    spawn_asset_thumbnail(ctx, asset_id.to_string(), picture);
    button
}

/// Helper to asynchronously request and bind an asset cover art thumbnail image.
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

/// Helper to asynchronously request and render a round avatar person face thumbnail.
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
