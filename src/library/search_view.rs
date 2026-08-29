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
    // --- Text ---
    pub filename_row: libadwaita::EntryRow,
    pub description_row: libadwaita::EntryRow,
    pub ocr_row: libadwaita::EntryRow,
    // --- Type & flags ---
    pub type_row: libadwaita::ComboRow,
    pub favorite_row: libadwaita::SwitchRow,
    pub archived_row: libadwaita::SwitchRow,
    pub motion_row: libadwaita::SwitchRow,
    pub not_in_album_row: libadwaita::SwitchRow,
    pub with_deleted_row: libadwaita::SwitchRow,
    pub visibility_row: libadwaita::ComboRow,
    pub rating_row: libadwaita::ComboRow,
    // --- Date range ---
    pub taken_after_row: libadwaita::EntryRow,
    pub taken_before_row: libadwaita::EntryRow,
    pub created_after_row: libadwaita::EntryRow,
    pub created_before_row: libadwaita::EntryRow,
    // --- Camera ---
    pub make_row: libadwaita::EntryRow,
    pub model_row: libadwaita::EntryRow,
    pub lens_row: libadwaita::EntryRow,
    // --- Location ---
    pub country_row: libadwaita::EntryRow,
    pub state_row: libadwaita::EntryRow,
    pub city_row: libadwaita::EntryRow,
    // --- Action ---
    pub search_button: gtk::Button,
    pub clear_button: gtk::Button,
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
fn build_filter_panel() -> (
    gtk::Revealer,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::ComboRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::ComboRow,
    libadwaita::ComboRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
) {
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .build();

    let filter_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(4)
        .build();

    let (text_group, filename_row, description_row, ocr_row) = build_text_group();
    filter_box.append(&text_group);

    let (flags_group, type_row, fav, archived, motion, not_album, deleted, vis, rating) =
        build_flags_group();
    filter_box.append(&flags_group);

    let (date_group, taken_after, taken_before, created_after, created_before) = build_date_group();
    filter_box.append(&date_group);

    let (camera_group, make_row, model_row, lens_row) = build_camera_group();
    filter_box.append(&camera_group);

    let (loc_group, country, state, city) = build_location_group();
    filter_box.append(&loc_group);

    revealer.set_child(Some(&filter_box));

    (
        revealer,
        filename_row,
        description_row,
        ocr_row,
        type_row,
        fav,
        archived,
        motion,
        not_album,
        deleted,
        vis,
        rating,
        taken_after,
        taken_before,
        created_after,
        created_before,
        make_row,
        model_row,
        lens_row,
        country,
        state,
        city,
    )
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
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(320)
        .css_classes(["mimick-search-scroll"])
        .build();

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

    let (
        filters_revealer,
        filename_row,
        description_row,
        ocr_row,
        type_row,
        favorite_row,
        archived_row,
        motion_row,
        not_in_album_row,
        with_deleted_row,
        visibility_row,
        rating_row,
        taken_after_row,
        taken_before_row,
        created_after_row,
        created_before_row,
        make_row,
        model_row,
        lens_row,
        country_row,
        state_row,
        city_row,
    ) = build_filter_panel();
    inner.append(&filters_revealer);

    scrolled.set_child(Some(&inner));
    connect_filter_toggle(&toggle_button, toggle_icon, filters_revealer);

    let root = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .child(&scrolled)
        .build();

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
        filename_row,
        description_row,
        ocr_row,
        type_row,
        favorite_row,
        archived_row,
        motion_row,
        not_in_album_row,
        with_deleted_row,
        visibility_row,
        rating_row,
        taken_after_row,
        taken_before_row,
        created_after_row,
        created_before_row,
        make_row,
        model_row,
        lens_row,
        country_row,
        state_row,
        city_row,
        search_button,
        clear_button,
    }
}

// ---------------------------------------------------------------------------
// Filter group builders (kept small to stay under cognitive-complexity limit)
// ---------------------------------------------------------------------------

fn build_text_group() -> (
    libadwaita::PreferencesGroup,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
) {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Text")
        .build();
    let filename = libadwaita::EntryRow::builder()
        .title("Filename contains")
        .build();
    let desc = libadwaita::EntryRow::builder()
        .title("Description contains")
        .build();
    let ocr = libadwaita::EntryRow::builder()
        .title("OCR text contains")
        .build();
    group.add(&filename);
    group.add(&desc);
    group.add(&ocr);
    (group, filename, desc, ocr)
}

fn combo_row(title: &str, items: &[&str]) -> libadwaita::ComboRow {
    libadwaita::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(items))
        .build()
}

#[allow(clippy::type_complexity)]
fn build_flags_group() -> (
    libadwaita::PreferencesGroup,
    libadwaita::ComboRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::SwitchRow,
    libadwaita::ComboRow,
    libadwaita::ComboRow,
) {
    fn switch(t: &str) -> libadwaita::SwitchRow {
        libadwaita::SwitchRow::builder().title(t).build()
    }

    let group = libadwaita::PreferencesGroup::builder()
        .title("Type and flags")
        .build();
    let type_row = combo_row("Asset type", &["Any", "Image only", "Video only"]);
    let fav = switch("Favourites only");
    let archived = switch("Archived only");
    let motion = switch("Motion photos only");
    let not_album = switch("Not in any album");
    let deleted = switch("Include deleted");
    let vis = combo_row(
        "Visibility",
        &["Any", "Timeline", "Archive", "Hidden", "Locked"],
    );
    let rating = combo_row("Minimum rating", &["Any", "1+", "2+", "3+", "4+", "5"]);
    group.add(&type_row);
    group.add(&fav);
    group.add(&archived);
    group.add(&motion);
    group.add(&not_album);
    group.add(&deleted);
    group.add(&vis);
    group.add(&rating);
    (
        group, type_row, fav, archived, motion, not_album, deleted, vis, rating,
    )
}

fn build_date_group() -> (
    libadwaita::PreferencesGroup,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
) {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Date range")
        .description("e.g. 2024-01-15")
        .build();
    let taken_after = libadwaita::EntryRow::builder().title("Taken after").build();
    let taken_before = libadwaita::EntryRow::builder()
        .title("Taken before")
        .build();
    let created_after = libadwaita::EntryRow::builder()
        .title("Created after")
        .build();
    let created_before = libadwaita::EntryRow::builder()
        .title("Created before")
        .build();
    group.add(&taken_after);
    group.add(&taken_before);
    group.add(&created_after);
    group.add(&created_before);
    (
        group,
        taken_after,
        taken_before,
        created_after,
        created_before,
    )
}

fn build_camera_group() -> (
    libadwaita::PreferencesGroup,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
) {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Camera")
        .build();
    let make = libadwaita::EntryRow::builder().title("Make").build();
    let model = libadwaita::EntryRow::builder().title("Model").build();
    let lens = libadwaita::EntryRow::builder().title("Lens model").build();
    group.add(&make);
    group.add(&model);
    group.add(&lens);
    (group, make, model, lens)
}

fn build_location_group() -> (
    libadwaita::PreferencesGroup,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
    libadwaita::EntryRow,
) {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Location")
        .build();
    let country = libadwaita::EntryRow::builder().title("Country").build();
    let state = libadwaita::EntryRow::builder()
        .title("State / region")
        .build();
    let city = libadwaita::EntryRow::builder().title("City").build();
    group.add(&country);
    group.add(&state);
    group.add(&city);
    (group, country, state, city)
}

// ---------------------------------------------------------------------------
// Filter extraction helpers
// ---------------------------------------------------------------------------

/// Collect all filter widget values into a `MetadataSearchFilters`.
pub fn collect_filters(view: &SearchViewParts) -> MetadataSearchFilters {
    MetadataSearchFilters {
        original_file_name: opt_string(&view.filename_row.text()),
        description: opt_string(&view.description_row.text()),
        ocr: opt_string(&view.ocr_row.text()),
        asset_type: match view.type_row.selected() {
            1 => Some("IMAGE".into()),
            2 => Some("VIDEO".into()),
            _ => None,
        },
        taken_after: normalise_iso_date(&view.taken_after_row.text()),
        taken_before: normalise_iso_date(&view.taken_before_row.text()),
        created_after: normalise_iso_date(&view.created_after_row.text()),
        created_before: normalise_iso_date(&view.created_before_row.text()),
        make: opt_string(&view.make_row.text()),
        model: opt_string(&view.model_row.text()),
        lens_model: opt_string(&view.lens_row.text()),
        country: opt_string(&view.country_row.text()),
        state: opt_string(&view.state_row.text()),
        city: opt_string(&view.city_row.text()),
        is_favorite: opt_true(view.favorite_row.is_active()),
        is_archived: opt_true(view.archived_row.is_active()),
        is_motion: opt_true(view.motion_row.is_active()),
        is_not_in_album: opt_true(view.not_in_album_row.is_active()),
        with_deleted: opt_true(view.with_deleted_row.is_active()),
        visibility: match view.visibility_row.selected() {
            1 => Some("timeline".into()),
            2 => Some("archive".into()),
            3 => Some("hidden".into()),
            4 => Some("locked".into()),
            _ => None,
        },
        rating: match view.rating_row.selected() {
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
    view.filename_row.set_text("");
    view.description_row.set_text("");
    view.ocr_row.set_text("");
    view.type_row.set_selected(0);
    view.favorite_row.set_active(false);
    view.archived_row.set_active(false);
    view.motion_row.set_active(false);
    view.not_in_album_row.set_active(false);
    view.with_deleted_row.set_active(false);
    view.visibility_row.set_selected(0);
    view.rating_row.set_selected(0);
    view.taken_after_row.set_text("");
    view.taken_before_row.set_text("");
    view.created_after_row.set_text("");
    view.created_before_row.set_text("");
    view.make_row.set_text("");
    view.model_row.set_text("");
    view.lens_row.set_text("");
    view.country_row.set_text("");
    view.state_row.set_text("");
    view.city_row.set_text("");
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
