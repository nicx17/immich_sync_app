use libadwaita::prelude::*;

pub struct FilterWidgets {
    pub revealer: gtk::Revealer,
    pub filename_row: libadwaita::EntryRow,
    pub description_row: libadwaita::EntryRow,
    pub ocr_row: libadwaita::EntryRow,
    pub type_row: libadwaita::ComboRow,
    pub favorite_row: libadwaita::SwitchRow,
    pub archived_row: libadwaita::SwitchRow,
    pub motion_row: libadwaita::SwitchRow,
    pub not_in_album_row: libadwaita::SwitchRow,
    pub with_deleted_row: libadwaita::SwitchRow,
    pub visibility_row: libadwaita::ComboRow,
    pub rating_row: libadwaita::ComboRow,
    pub taken_after_row: libadwaita::EntryRow,
    pub taken_before_row: libadwaita::EntryRow,
    pub created_after_row: libadwaita::EntryRow,
    pub created_before_row: libadwaita::EntryRow,
    pub make_row: libadwaita::EntryRow,
    pub model_row: libadwaita::EntryRow,
    pub lens_row: libadwaita::EntryRow,
    pub country_row: libadwaita::EntryRow,
    pub state_row: libadwaita::EntryRow,
    pub city_row: libadwaita::EntryRow,
}

pub(super) fn build_text_group() -> (
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
pub(super) fn build_flags_group() -> (
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

pub(super) fn build_date_group() -> (
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

pub(super) fn build_camera_group() -> (
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

pub(super) fn build_location_group() -> (
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
