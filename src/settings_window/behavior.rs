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

    let startup_row = add_switch(&behavior_group, "Run on Startup", "Start Mimick at login.");
    let background_sync_row = add_switch(
        &behavior_group,
        "Background Sync",
        "Watch folders in the background after launch.",
    );
    let metered_row = add_switch(
        &behavior_group,
        "Pause on Metered Network",
        "Pause uploads on metered connections.",
    );
    let battery_row = add_switch(
        &behavior_group,
        "Pause on Battery Power",
        "Pause uploads while on battery.",
    );
    let notifications_row = add_switch(
        &behavior_group,
        "Enable Notifications",
        "Desktop notifications for sync events.",
    );
    let library_view_row = add_switch(
        &behavior_group,
        "Enable Library View",
        "In-app library browser. Requires restart.",
    );
    let catchup_row = add_catchup_row(&behavior_group);
    let concurrency_row = add_spin(
        &behavior_group,
        "Upload Workers",
        "Parallel uploads. More = faster batches.",
        gtk::Adjustment::new(3.0, 1.0, 10.0, 1.0, 1.0, 0.0),
    );
    let xmp_sidecar_row = add_switch(
        &behavior_group,
        "Upload XMP Sidecars",
        "Attach .xmp sidecars with uploads.",
    );
    let quiet_hours_row = add_switch(
        &behavior_group,
        "Quiet Hours",
        "Pause uploads on a nightly schedule.",
    );
    let quiet_start_row = add_spin(
        &behavior_group,
        "Quiet Hours Start (hour, local)",
        "",
        gtk::Adjustment::new(22.0, 0.0, 23.0, 1.0, 1.0, 0.0),
    );
    let quiet_end_row = add_spin(
        &behavior_group,
        "Quiet Hours End (hour, local)",
        "",
        gtk::Adjustment::new(7.0, 0.0, 23.0, 1.0, 1.0, 0.0),
    );

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
