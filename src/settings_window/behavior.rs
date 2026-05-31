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

    let startup_row = adw::SwitchRow::builder()
        .title("Run on Startup")
        .subtitle("Start Mimick at login.")
        .build();
    behavior_group.add(&startup_row);

    let background_sync_row = adw::SwitchRow::builder()
        .title("Background Sync")
        .subtitle("Watch folders in the background after launch.")
        .build();
    behavior_group.add(&background_sync_row);

    let metered_row = adw::SwitchRow::builder()
        .title("Pause on Metered Network")
        .subtitle("Pause uploads on metered connections.")
        .build();
    behavior_group.add(&metered_row);

    let battery_row = adw::SwitchRow::builder()
        .title("Pause on Battery Power")
        .subtitle("Pause uploads while on battery.")
        .build();
    behavior_group.add(&battery_row);

    let notifications_row = adw::SwitchRow::builder()
        .title("Enable Notifications")
        .subtitle("Desktop notifications for sync events.")
        .build();
    behavior_group.add(&notifications_row);

    let library_view_row = adw::SwitchRow::builder()
        .title("Enable Library View")
        .subtitle("In-app library browser. Requires restart.")
        .build();
    behavior_group.add(&library_view_row);

    let catchup_model = gtk::StringList::new(&["Full Scan", "Recent Only (7d)", "New Files Only"]);
    let catchup_row = adw::ComboRow::builder()
        .title("Default Startup Catch-up Mode")
        .subtitle("Used when a folder has no override.")
        .model(&catchup_model)
        .build();
    behavior_group.add(&catchup_row);

    // Upload concurrency (1–10 workers)
    let concurrency_adj = gtk::Adjustment::new(3.0, 1.0, 10.0, 1.0, 1.0, 0.0);
    let concurrency_row = adw::SpinRow::builder()
        .title("Upload Workers")
        .subtitle("Parallel uploads. More = faster batches.")
        .adjustment(&concurrency_adj)
        .build();
    behavior_group.add(&concurrency_row);

    let xmp_sidecar_row = adw::SwitchRow::builder()
        .title("Upload XMP Sidecars")
        .subtitle("Attach .xmp sidecars with uploads.")
        .build();
    behavior_group.add(&xmp_sidecar_row);

    let quiet_hours_row = adw::SwitchRow::builder()
        .title("Quiet Hours")
        .subtitle("Pause uploads on a nightly schedule.")
        .build();
    behavior_group.add(&quiet_hours_row);

    let quiet_start_adj = gtk::Adjustment::new(22.0, 0.0, 23.0, 1.0, 1.0, 0.0);
    let quiet_start_row = adw::SpinRow::builder()
        .title("Quiet Hours Start (hour, local)")
        .adjustment(&quiet_start_adj)
        .build();
    behavior_group.add(&quiet_start_row);

    let quiet_end_adj = gtk::Adjustment::new(7.0, 0.0, 23.0, 1.0, 1.0, 0.0);
    let quiet_end_row = adw::SpinRow::builder()
        .title("Quiet Hours End (hour, local)")
        .adjustment(&quiet_end_adj)
        .build();
    behavior_group.add(&quiet_end_row);

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
