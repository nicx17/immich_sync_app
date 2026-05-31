use adw::prelude::*;
use gtk::ProgressBar;
use libadwaita as adw;

pub struct StatusWidgets {
    pub status_row: adw::ActionRow,
    pub progress_bar: ProgressBar,
    pub route_row: adw::ActionRow,
    pub folders_row: adw::ActionRow,
    pub queue_health_row: adw::ActionRow,
    pub last_sync_row: adw::ActionRow,
    pub error_row: adw::ActionRow,
}

pub fn build_status_group(status_page: &adw::PreferencesPage) -> StatusWidgets {
    let progress_group = adw::PreferencesGroup::builder()
        .title("Sync Status")
        .build();
    status_page.add(&progress_group);

    let status_row = action_row("Idle", "Waiting to sync...");
    progress_group.add(&status_row);

    let progress_bar = progress_bar();
    progress_group.add(&progress_bar);

    let health_group = adw::PreferencesGroup::builder()
        .title("Health Dashboard")
        .build();
    status_page.add(&health_group);

    let route_row = action_row("Server Route", "Checking connectivity...");
    health_group.add(&route_row);

    let folders_row = action_row("Watched Folders", "0 configured");
    health_group.add(&folders_row);

    let queue_health_row = action_row("Queue Health", "0 pending, 0 waiting to retry");
    health_group.add(&queue_health_row);

    let last_sync_row = action_row("Last Successful Sync", "No successful sync yet");
    health_group.add(&last_sync_row);

    let error_row = action_row("No recent errors", "Uploads are healthy.");
    health_group.add(&error_row);

    StatusWidgets {
        status_row,
        progress_bar,
        route_row,
        folders_row,
        queue_health_row,
        last_sync_row,
        error_row,
    }
}

fn action_row(title: &str, subtitle: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build()
}

fn progress_bar() -> ProgressBar {
    ProgressBar::builder()
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .fraction(0.0)
        .build()
}
