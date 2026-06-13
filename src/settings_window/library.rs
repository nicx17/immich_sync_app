use adw::prelude::*;
use libadwaita as adw;

pub struct LibraryWidgets {
    pub library_group: adw::PreferencesGroup,
    pub preview_full_row: adw::SwitchRow,
    pub grid_quality_row: adw::ComboRow,
    pub raw_full_decode_row: adw::SwitchRow,
    pub raw_cache_row: adw::SwitchRow,
    pub disk_cache_row: adw::SpinRow,
    pub download_folder_row: adw::ActionRow,
    pub download_change_btn: gtk::Button,
    pub download_clear_btn: gtk::Button,
}

pub fn build_library_group(settings_page: &adw::PreferencesPage) -> LibraryWidgets {
    let library_group = adw::PreferencesGroup::builder()
        .title("Library")
        .description("Library browser settings.")
        .build();
    settings_page.add(&library_group);

    let preview_full_row = adw::SwitchRow::builder()
        .title("Open Originals in Lightbox")
        .subtitle("Use full-resolution instead of the 1440px preview.")
        .build();
    library_group.add(&preview_full_row);

    let quality_options =
        gtk::StringList::new(&["Auto (per cell size)", "Thumbnail", "Preview", "Full Size"]);
    let grid_quality_row = adw::ComboRow::builder()
        .title("Library Thumbnail Quality")
        .subtitle("Higher quality uses more memory.")
        .model(&quality_options)
        .build();
    library_group.add(&grid_quality_row);

    let raw_full_decode_row = adw::SwitchRow::builder()
        .title("Full RAW Decoding")
        .subtitle("Sensor data instead of embedded previews. Slower.")
        .build();
    library_group.add(&raw_full_decode_row);

    let raw_cache_row = adw::SwitchRow::builder()
        .title("Cache Decoded RAW Files")
        .subtitle("Instant re-opens. Uses disk space.")
        .build();
    library_group.add(&raw_cache_row);

    let disk_cache_adj = gtk::Adjustment::new(2000.0, 200.0, 10000.0, 100.0, 500.0, 0.0);
    let disk_cache_row = adw::SpinRow::builder()
        .title("Disk Cache Size (MB)")
        .subtitle("Cap across all on-disk caches. Pruned at startup.")
        .adjustment(&disk_cache_adj)
        .build();
    library_group.add(&disk_cache_row);

    let download_folder_row = adw::ActionRow::builder()
        .title("Download Folder")
        .subtitle("Not set")
        .subtitle_lines(1)
        .build();
    let download_change_btn = gtk::Button::builder()
        .label("Change")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let download_clear_btn = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("Clear saved folder")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .visible(false)
        .build();
    download_folder_row.add_suffix(&download_clear_btn);
    download_folder_row.add_suffix(&download_change_btn);
    library_group.add(&download_folder_row);

    LibraryWidgets {
        library_group,
        preview_full_row,
        grid_quality_row,
        raw_full_decode_row,
        raw_cache_row,
        disk_cache_row,
        download_folder_row,
        download_change_btn,
        download_clear_btn,
    }
}
