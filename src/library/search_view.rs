//! Dedicated search view for the library sidebar.
//!
//! The search form is a persistent `Revealer` that sits above the
//! content_stack.  When search is active the form stays visible and
//! results render in the normal masonry grid below it.  This avoids
//! the "search bar disappears" problem while keeping a single grid
//! renderer.

use std::cell::Cell;
use std::rc::Rc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::MetadataSearchFilters;
use crate::library::search_filters::{
    FilterWidgets, build_camera_group, build_date_group, build_flags_group, build_location_group,
    build_text_group,
};

/// UI widgets for the dedicated search form.
///
/// `root` is a `gtk::Revealer` intended to sit between the controls
/// bar and the content_stack in the main layout.  When revealed, it
/// shows the search entry, mode selector, collapsible filters, and
/// action buttons.  Search results go into the existing grid.
pub struct SearchViewParts {
    /// Revealer wrapping the entire search form.  Inserted into the
    /// content pane above the content_stack.
    pub root: gtk::Revealer,
    pub search_entry: gtk::SearchEntry,
    pub search_mode: gtk::DropDown,
    pub search_button: gtk::Button,
    pub clear_button: gtk::Button,
    pub filters: FilterWidgets,
}

fn build_search_bar() -> (gtk::Box, gtk::DropDown, gtk::SearchEntry) {
    let mode_model = gtk::StringList::new(&["Smart Search", "Filename", "OCR"]);
    let search_mode = gtk::DropDown::builder()
        .model(&mode_model)
        .selected(0)
        .tooltip_text(
            "Smart: CLIP-based semantic search.\n\
             Filename: matches against file names.\n\
             OCR: matches text inside images.",
        )
        .build();

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Describe what you're looking for\u{2026}")
        .hexpand(true)
        .build();

    let search_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    search_bar.append(&search_mode);
    search_bar.append(&search_entry);

    (search_bar, search_mode, search_entry)
}

fn build_action_row() -> (gtk::Box, gtk::Image, gtk::Button, gtk::Button, gtk::Button) {
    let action_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(2)
        .build();

    let toggle_icon = gtk::Image::from_icon_name("pan-end-symbolic");
    let toggle_button = gtk::Button::builder()
        .child(&{
            let b = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            b.append(&toggle_icon);
            b.append(
                &gtk::Label::builder()
                    .label("Filters")
                    .css_classes(vec!["caption".to_string()])
                    .build(),
            );
            b
        })
        .css_classes(vec!["flat".to_string()])
        .build();

    let clear_button = gtk::Button::builder()
        .label("Clear")
        .css_classes(vec!["flat".to_string()])
        .build();
    let search_button = gtk::Button::builder()
        .label("Search")
        .css_classes(vec!["suggested-action".to_string()])
        .build();

    action_row.append(&toggle_button);
    // Spacer pushes buttons to the right.
    action_row.append(&gtk::Box::builder().hexpand(true).build());
    action_row.append(&clear_button);
    action_row.append(&search_button);

    (
        action_row,
        toggle_icon,
        toggle_button,
        clear_button,
        search_button,
    )
}

/// Assemble all filter groups into a revealer.
#[allow(clippy::type_complexity)]
fn build_filter_panel() -> FilterWidgets {
    let (text_group, filename_row, description_row, ocr_row) = build_text_group();
    let (flags_group, type_row, fav, archived, motion, not_album, deleted, vis, rating) =
        build_flags_group();
    let (date_group, taken_after, taken_before, created_after, created_before) = build_date_group();
    let (camera_group, make_row, model_row, lens_row) = build_camera_group();
    let (loc_group, country, state, city) = build_location_group();

    let filter_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(4)
        .build();
    filter_box.append(&text_group);
    filter_box.append(&flags_group);
    filter_box.append(&date_group);
    filter_box.append(&camera_group);
    filter_box.append(&loc_group);

    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .child(&filter_box)
        .build();

    FilterWidgets {
        revealer,
        filename_row,
        description_row,
        ocr_row,
        type_row,
        favorite_row: fav,
        archived_row: archived,
        motion_row: motion,
        not_in_album_row: not_album,
        with_deleted_row: deleted,
        visibility_row: vis,
        rating_row: rating,
        taken_after_row: taken_after,
        taken_before_row: taken_before,
        created_after_row: created_after,
        created_before_row: created_before,
        make_row,
        model_row,
        lens_row,
        country_row: country,
        state_row: state,
        city_row: city,
    }
}

/// Wire the toggle button to show/hide the filters revealer.
fn connect_filter_toggle(
    toggle_button: &gtk::Button,
    toggle_icon: gtk::Image,
    revealer: gtk::Revealer,
) {
    let expanded = Rc::new(Cell::new(false));
    toggle_button.connect_clicked(move |_| {
        let now = !expanded.get();
        expanded.set(now);
        revealer.set_reveal_child(now);
        toggle_icon.set_icon_name(Some(if now {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    });
}

/// Build the search form widget tree.
///
/// Returns a `SearchViewParts` whose `root` is a `Revealer`.  The
/// caller should insert `root` into the content pane above the
/// `content_stack` so the form stays visible while results load into
/// the grid.
pub fn build_search_view() -> SearchViewParts {
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(8)
        .margin_bottom(4)
        .margin_start(8)
        .margin_end(8)
        .build();

    let (search_bar, search_mode, search_entry) = build_search_bar();
    inner.append(&search_bar);

    let (action_row, toggle_icon, toggle_button, clear_button, search_button) = build_action_row();
    inner.append(&action_row);

    let filters = build_filter_panel();
    inner.append(&filters.revealer);

    let root = wrap_in_scrolled_revealer(&inner);
    connect_filter_toggle(&toggle_button, toggle_icon, filters.revealer.clone());

    search_mode.connect_selected_notify(clone!(
        #[weak]
        search_entry,
        move |dd| {
            let ph = match dd.selected() {
                1 => "Search filenames",
                2 => "Find words shown inside images",
                _ => "Describe what you're looking for\u{2026}",
            };
            search_entry.set_placeholder_text(Some(ph));
        }
    ));

    SearchViewParts {
        root,
        search_entry,
        search_mode,
        search_button,
        clear_button,
        filters,
    }
}

fn wrap_in_scrolled_revealer(inner: &gtk::Box) -> gtk::Revealer {
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(320)
        .css_classes(["mimick-search-scroll"])
        .child(inner)
        .build();

    gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .child(&scrolled)
        .build()
}

// ---------------------------------------------------------------------------
// Filter extraction helpers
// ---------------------------------------------------------------------------

/// Collect all filter widget values into a `MetadataSearchFilters`.
pub fn collect_filters(view: &SearchViewParts) -> MetadataSearchFilters {
    MetadataSearchFilters {
        original_file_name: opt_string(&view.filters.filename_row.text()),
        description: opt_string(&view.filters.description_row.text()),
        ocr: opt_string(&view.filters.ocr_row.text()),
        asset_type: match view.filters.type_row.selected() {
            1 => Some("IMAGE".into()),
            2 => Some("VIDEO".into()),
            _ => None,
        },
        taken_after: normalise_iso_date(&view.filters.taken_after_row.text()),
        taken_before: normalise_iso_date(&view.filters.taken_before_row.text()),
        created_after: normalise_iso_date(&view.filters.created_after_row.text()),
        created_before: normalise_iso_date(&view.filters.created_before_row.text()),
        make: opt_string(&view.filters.make_row.text()),
        model: opt_string(&view.filters.model_row.text()),
        lens_model: opt_string(&view.filters.lens_row.text()),
        country: opt_string(&view.filters.country_row.text()),
        state: opt_string(&view.filters.state_row.text()),
        city: opt_string(&view.filters.city_row.text()),
        is_favorite: opt_true(view.filters.favorite_row.is_active()),
        is_archived: opt_true(view.filters.archived_row.is_active()),
        is_motion: opt_true(view.filters.motion_row.is_active()),
        is_not_in_album: opt_true(view.filters.not_in_album_row.is_active()),
        with_deleted: opt_true(view.filters.with_deleted_row.is_active()),
        visibility: match view.filters.visibility_row.selected() {
            1 => Some("timeline".into()),
            2 => Some("archive".into()),
            3 => Some("hidden".into()),
            4 => Some("locked".into()),
            _ => None,
        },
        rating: match view.filters.rating_row.selected() {
            1 => Some(1),
            2 => Some(2),
            3 => Some(3),
            4 => Some(4),
            5 => Some(5),
            _ => None,
        },
        ..Default::default()
    }
}

/// Reset all filter widgets to their default (empty) state.
pub fn clear_all_filters(view: &SearchViewParts) {
    view.search_entry.set_text("");
    view.filters.filename_row.set_text("");
    view.filters.description_row.set_text("");
    view.filters.ocr_row.set_text("");
    view.filters.type_row.set_selected(0);
    view.filters.favorite_row.set_active(false);
    view.filters.archived_row.set_active(false);
    view.filters.motion_row.set_active(false);
    view.filters.not_in_album_row.set_active(false);
    view.filters.with_deleted_row.set_active(false);
    view.filters.visibility_row.set_selected(0);
    view.filters.rating_row.set_selected(0);
    view.filters.taken_after_row.set_text("");
    view.filters.taken_before_row.set_text("");
    view.filters.created_after_row.set_text("");
    view.filters.created_before_row.set_text("");
    view.filters.make_row.set_text("");
    view.filters.model_row.set_text("");
    view.filters.lens_row.set_text("");
    view.filters.country_row.set_text("");
    view.filters.state_row.set_text("");
    view.filters.city_row.set_text("");
}

/// Convert a non-empty trimmed string to `Some`, otherwise `None`.
pub fn opt_string(text: &gtk::glib::GString) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn opt_true(active: bool) -> Option<bool> {
    if active { Some(true) } else { None }
}

/// Normalise a user-entered date to ISO 8601 UTC. Accepts bare YYYY-MM-DD
/// or full RFC 3339.
pub fn normalise_iso_date(text: &gtk::glib::GString) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if chrono::DateTime::parse_from_rfc3339(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }
    if chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d").is_ok() {
        return Some(format!("{trimmed}T00:00:00.000Z"));
    }
    None
}
