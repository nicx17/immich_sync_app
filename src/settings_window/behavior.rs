use adw::prelude::*;
use libadwaita as adw;

pub struct BehaviorWidgets {
    pub startup_row: adw::SwitchRow,
    pub background_sync_row: adw::SwitchRow,
    pub metered_row: adw::SwitchRow,
    pub battery_row: adw::SwitchRow,
    pub notifications_row: adw::SwitchRow,
    pub library_view_row: adw::SwitchRow,
    pub catchup_row: adw::ComboRow,
    pub concurrency_row: adw::SpinRow,
    pub xmp_sidecar_row: adw::SwitchRow,
    pub quiet_hours_row: adw::SwitchRow,
    pub quiet_start_row: adw::SpinRow,
    pub quiet_end_row: adw::SpinRow,
}

pub fn build_behavior_group(settings_page: &adw::PreferencesPage) -> BehaviorWidgets {
    let behavior_group = adw::PreferencesGroup::builder().title("Behavior").build();
    settings_page.add(&behavior_group);

    let (
        startup_row,
        background_sync_row,
        metered_row,
        battery_row,
        notifications_row,
        library_view_row,
    ) = add_toggle_rows(&behavior_group);
    let (catchup_row, concurrency_row, xmp_sidecar_row) = add_upload_rows(&behavior_group);
    let (quiet_hours_row, quiet_start_row, quiet_end_row) = add_quiet_hours_rows(&behavior_group);

    BehaviorWidgets {
        startup_row,
        background_sync_row,
        metered_row,
        battery_row,
        notifications_row,
        library_view_row,
        catchup_row,
        concurrency_row,
        xmp_sidecar_row,
        quiet_hours_row,
        quiet_start_row,
        quiet_end_row,
    }
}

fn add_toggle_rows(
    group: &adw::PreferencesGroup,
) -> (
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
    adw::SwitchRow,
) {
    let startup = add_switch(group, "Run on Startup", "Start Mimick at login.");
    let background = add_switch(
        group,
        "Background Sync",
        "Watch folders in the background after launch.",
    );
    let metered = add_switch(
        group,
        "Pause on Metered Network",
        "Pause uploads on metered connections.",
    );
    let battery = add_switch(
        group,
        "Pause on Battery Power",
        "Pause uploads while on battery.",
    );
    let notifications = add_switch(
        group,
        "Enable Notifications",
        "Desktop notifications for sync events.",
    );
    let library = add_switch(
        group,
        "Enable Library View",
        "In-app library browser. Requires restart.",
    );
    (
        startup,
        background,
        metered,
        battery,
        notifications,
        library,
    )
}

fn add_upload_rows(group: &adw::PreferencesGroup) -> (adw::ComboRow, adw::SpinRow, adw::SwitchRow) {
    let catchup = add_catchup_row(group);
    let concurrency = add_spin(
        group,
        "Upload Workers",
        "Parallel uploads. More = faster batches.",
        gtk::Adjustment::new(3.0, 1.0, 10.0, 1.0, 1.0, 0.0),
    );
    let xmp = add_switch(
        group,
        "Upload XMP Sidecars",
        "Attach .xmp sidecars with uploads.",
    );
    (catchup, concurrency, xmp)
}

fn add_quiet_hours_rows(
    group: &adw::PreferencesGroup,
) -> (adw::SwitchRow, adw::SpinRow, adw::SpinRow) {
    let toggle = add_switch(group, "Quiet Hours", "Pause uploads on a nightly schedule.");
    let start = add_spin(
        group,
        "Quiet Hours Start (hour, local)",
        "",
        gtk::Adjustment::new(22.0, 0.0, 23.0, 1.0, 1.0, 0.0),
    );
    let end = add_spin(
        group,
        "Quiet Hours End (hour, local)",
        "",
        gtk::Adjustment::new(7.0, 0.0, 23.0, 1.0, 1.0, 0.0),
    );
    (toggle, start, end)
}

fn add_switch(group: &adw::PreferencesGroup, title: &str, subtitle: &str) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    group.add(&row);
    row
}

fn add_catchup_row(group: &adw::PreferencesGroup) -> adw::ComboRow {
    let model = gtk::StringList::new(&["Full Scan", "Recent Only (7d)", "New Files Only"]);
    let row = adw::ComboRow::builder()
        .title("Default Startup Catch-up Mode")
        .subtitle("Used when a folder has no override.")
        .model(&model)
        .build();
    group.add(&row);
    row
}

fn add_spin(
    group: &adw::PreferencesGroup,
    title: &str,
    subtitle: &str,
    adjustment: gtk::Adjustment,
) -> adw::SpinRow {
    let mut builder = adw::SpinRow::builder().title(title).adjustment(&adjustment);
    if !subtitle.is_empty() {
        builder = builder.subtitle(subtitle);
    }
    let row = builder.build();
    group.add(&row);
    row
}
