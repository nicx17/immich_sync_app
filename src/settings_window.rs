//! Implements the GTK4/Libadwaita settings window and status dashboard.

use crate::autostart;
use crate::config::{FolderRules, StartupCatchupMode, WatchPathEntry};
use crate::diagnostics;
use adw::prelude::*;
use glib::clone;
use gtk::prelude::*;
use gtk::{
    Box, Button, Entry, FileDialog, ListBox, Orientation, PasswordEntry, ProgressBar,
    ScrolledWindow, Switch,
};
use libadwaita as adw;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

use crate::api_client::ImmichApiClient;
use crate::config::Config;
use crate::monitor::MonitorHandle;
use crate::queue_manager::QueueManager;
use crate::state_manager::AppState;
use crate::watch_path_display::{display_watch_path, watch_path_subtitle};

/// Holds GTK widgets for a single watch-folder row in the settings list.
struct FolderRowData {
    path: String,
    album_name: Rc<RefCell<String>>,
    rules: Rc<RefCell<FolderRules>>,
    action_row: adw::ExpanderRow,
    base_subtitle: String,
}

const DEFAULT_ALBUM_LABEL: &str = "Default (Folder Name)";

fn format_sync_age(timestamp: Option<f64>) -> String {
    let Some(timestamp) = timestamp else {
        return "No successful sync yet".to_string();
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let elapsed = (now - timestamp).max(0.0);

    if elapsed < 60.0 {
        "Less than a minute ago".to_string()
    } else if elapsed < 3600.0 {
        format!("{} minute(s) ago", (elapsed / 60.0).floor() as u64)
    } else if elapsed < 86_400.0 {
        format!("{} hour(s) ago", (elapsed / 3600.0).floor() as u64)
    } else {
        format!("{} day(s) ago", (elapsed / 86_400.0).floor() as u64)
    }
}

/// Build the main settings window and wire it to the shared app state.
pub fn build_settings_window(
    app: &adw::Application,
    shared_state: Arc<Mutex<AppState>>,
    api_client: Option<Arc<ImmichApiClient>>,
    queue_manager: Option<Arc<QueueManager>>,
    monitor_handle: Option<Arc<MonitorHandle>>,
    sync_now_tx: Option<UnboundedSender<()>>,
) {
    // Use a PreferencesWindow for native Libadwaita mobile responsiveness and adaptive layouts
    let window = adw::PreferencesWindow::builder()
        .application(app)
        .search_enabled(false)
        .default_width(660)
        .default_height(840)
        .build();
    let app_clone = app.clone();
    let config = Config::new();

    let status_page = adw::PreferencesPage::builder()
        .title("Status")
        .icon_name("dialog-information-symbolic")
        .build();
    window.add(&status_page);

    let settings_page = adw::PreferencesPage::builder()
        .title("Settings")
        .icon_name("emblem-system-symbolic")
        .build();
    window.add(&settings_page);

    let is_unconfigured = config.get_api_key().unwrap_or_default().is_empty();
    if is_unconfigured {
        let welcome_group = adw::PreferencesGroup::builder()
            .title("Welcome to Mimick!")
            .description("Start by adding your API key, testing the connection, and choosing at least one folder.")
            .build();

        let help_row = adw::ActionRow::builder()
            .title("How to get an API Key")
            .activatable(true)
            .build();

        help_row.connect_activated(|_| {
            let uri = "https://immich.app/docs/features/command-line-interface/#api-key";
            if let Err(e) =
                gtk::gio::AppInfo::launch_default_for_uri(uri, None::<&gtk::gio::AppLaunchContext>)
            {
                log::error!("Failed to open browser: {}", e);
            }
        });

        welcome_group.add(&help_row);
        settings_page.add(&welcome_group);
    }

    // --- PROGRESS GROUP ---
    let progress_group = adw::PreferencesGroup::builder()
        .title("Sync Status")
        .build();
    status_page.add(&progress_group);

    let status_row = adw::ActionRow::builder()
        .title("Idle")
        .subtitle("Waiting to sync...")
        .build();
    progress_group.add(&status_row);

    let progress_bar = ProgressBar::builder()
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .fraction(0.0)
        .build();
    progress_group.add(&progress_bar);

    let health_group = adw::PreferencesGroup::builder()
        .title("Health Dashboard")
        .build();
    status_page.add(&health_group);

    let route_row = adw::ActionRow::builder()
        .title("Server Route")
        .subtitle("Checking connectivity...")
        .build();
    health_group.add(&route_row);

    let folders_row = adw::ActionRow::builder()
        .title("Watched Folders")
        .subtitle("0 configured")
        .build();
    health_group.add(&folders_row);

    let queue_health_row = adw::ActionRow::builder()
        .title("Queue Health")
        .subtitle("0 pending, 0 waiting to retry")
        .build();
    health_group.add(&queue_health_row);

    let last_sync_row = adw::ActionRow::builder()
        .title("Last Successful Sync")
        .subtitle("No successful sync yet")
        .build();
    health_group.add(&last_sync_row);

    let error_row = adw::ActionRow::builder()
        .title("No recent errors")
        .subtitle("Uploads are healthy.")
        .build();
    health_group.add(&error_row);

    // --- CONNECTIVITY GROUP ---
    let conn_group = adw::PreferencesGroup::builder()
        .title("Connectivity")
        .build();
    settings_page.add(&conn_group);

    // Internal URL
    let internal_row = adw::ActionRow::builder()
        .title("Internal URL (LAN)")
        .build();
    let internal_switch = Switch::builder().valign(gtk::Align::Center).build();
    let internal_entry = Entry::builder()
        .placeholder_text("http://192.168.1.10:2283")
        .valign(gtk::Align::Center)
        .width_request(0)
        .hexpand(true)
        .build();
    internal_row.add_prefix(&internal_switch);
    internal_row.add_suffix(&internal_entry);
    conn_group.add(&internal_row);

    // External URL
    let external_row = adw::ActionRow::builder()
        .title("External URL (WAN)")
        .build();
    let external_switch = Switch::builder().valign(gtk::Align::Center).build();
    let external_entry = Entry::builder()
        .placeholder_text("https://immich.example.com")
        .valign(gtk::Align::Center)
        .width_request(0)
        .hexpand(true)
        .build();
    external_row.add_prefix(&external_switch);
    external_row.add_suffix(&external_entry);
    conn_group.add(&external_row);

    // Toggle validation: prevent both switches being OFF at the same time
    // Mirrors Python's _validate_toggles logic
    internal_switch.connect_active_notify(clone!(
        #[weak]
        external_switch,
        #[weak]
        window,
        move |sw| {
            if !sw.is_active() && !external_switch.is_active() {
                sw.set_active(true);
                let dialog = gtk::AlertDialog::builder()
                    .message("At least one URL required")
                    .detail("You must keep at least one URL switch enabled.")
                    .buttons(["OK"])
                    .build();
                dialog.show(Some(&window));
            }
        }
    ));

    external_switch.connect_active_notify(clone!(
        #[weak]
        internal_switch,
        #[weak]
        window,
        move |sw| {
            if !sw.is_active() && !internal_switch.is_active() {
                sw.set_active(true);
                let dialog = gtk::AlertDialog::builder()
                    .message("At least one URL required")
                    .detail("You must keep at least one URL switch enabled.")
                    .buttons(["OK"])
                    .build();
                dialog.show(Some(&window));
            }
        }
    ));

    // API Key
    let api_key_row = adw::ActionRow::builder().title("API Key").build();
    let api_key_entry = PasswordEntry::builder()
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    api_key_row.add_suffix(&api_key_entry);
    conn_group.add(&api_key_row);

    // Test Connection Button
    let test_btn = Button::builder()
        .label("Test Connection")
        .margin_top(12)
        .build();
    conn_group.add(&test_btn);

    // Clone before moving into test_btn closure so api_client is still available below
    let api_client_for_test = api_client.clone();
    test_btn.connect_clicked(clone!(
        #[weak]
        internal_switch,
        #[weak]
        external_switch,
        #[weak]
        internal_entry,
        #[weak]
        external_entry,
        #[weak]
        api_key_entry,
        #[weak]
        window,
        #[weak]
        test_btn,
        move |btn| {
            btn.set_sensitive(false);

            // Collect only primitive/String values – no GTK types cross threads
            let internal = if internal_switch.is_active() {
                internal_entry.text().to_string()
            } else {
                String::new()
            };
            let external = if external_switch.is_active() {
                external_entry.text().to_string()
            } else {
                String::new()
            };
            let _api_key = api_key_entry.text().to_string();

            let (tx, mut rx) = tokio::sync::oneshot::channel::<(bool, bool)>();

            // Use the application-wide API client — do NOT create ImmichApiClient::new() here.
            // Creating a fresh reqwest client per click allocates a new connection pool
            // that lingers for 30s even after the test completes.
            if let Some(ref shared_client) = api_client_for_test {
                let ping_client = shared_client.clone();
                let internal2 = internal.clone();
                let external2 = external.clone();
                tokio::spawn(async move {
                    let int_ok = if !internal2.is_empty() {
                        ping_client.ping_url(&internal2).await
                    } else {
                        false
                    };
                    let ext_ok = if !external2.is_empty() {
                        ping_client.ping_url(&external2).await
                    } else {
                        false
                    };
                    let _ = tx.send((int_ok, ext_ok));
                });
            } else {
                // No client available — report failure
                let _ = tx.send((false, false));
            }

            // Poll the oneshot receiver from the GTK main loop
            glib::timeout_add_local(
                Duration::from_millis(50),
                clone!(
                    #[weak]
                    window,
                    #[weak]
                    test_btn,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        match rx.try_recv() {
                            Ok((int_ok, ext_ok)) => {
                                test_btn.set_sensitive(true);

                                let int_label = if int_ok { "OK" } else { "FAILED" };
                                let ext_label = if ext_ok { "OK" } else { "FAILED" };
                                let mut report =
                                    format!("Internal: {}\nExternal: {}", int_label, ext_label);
                                let heading = if int_ok || ext_ok {
                                    if int_ok {
                                        report.push_str("\n\nActive Mode: LAN");
                                    } else {
                                        report.push_str("\n\nActive Mode: WAN");
                                    }
                                    "Connection Successful"
                                } else {
                                    report = "Could not connect to Immich at either address."
                                        .to_string();
                                    "Connection Failed"
                                };

                                let dialog = adw::MessageDialog::builder()
                                    .transient_for(&window)
                                    .heading(heading)
                                    .body(&report)
                                    .build();
                                dialog.add_response("ok", "OK");
                                dialog.present();

                                glib::ControlFlow::Break
                            }
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                                // Still waiting
                                glib::ControlFlow::Continue
                            }
                            Err(_) => glib::ControlFlow::Break, // channel dropped
                        }
                    }
                ),
            );
        }
    ));

    let behavior_group = adw::PreferencesGroup::builder().title("Behavior").build();
    settings_page.add(&behavior_group);

    let startup_row = adw::SwitchRow::builder()
        .title("Run on Startup")
        .subtitle("Start Mimick automatically when you log in.")
        .build();
    behavior_group.add(&startup_row);

    let metered_row = adw::SwitchRow::builder()
        .title("Pause on Metered Network")
        .subtitle("Defer uploads while the active connection is marked as metered.")
        .build();
    behavior_group.add(&metered_row);

    let battery_row = adw::SwitchRow::builder()
        .title("Pause on Battery Power")
        .subtitle("Defer uploads while the system appears to be running on battery.")
        .build();
    behavior_group.add(&battery_row);

    let notifications_row = adw::SwitchRow::builder()
        .title("Enable Notifications")
        .subtitle("Show desktop notifications for sync events and connectivity issues.")
        .build();
    behavior_group.add(&notifications_row);

    let catchup_model = gtk::StringList::new(&["Full Scan", "Recent Only (7d)", "New Files Only"]);
    let catchup_row = adw::ComboRow::builder()
        .title("Startup Catch-up Mode")
        .subtitle("Limits how aggressively Mimick scans for changes when launching.")
        .model(&catchup_model)
        .build();
    behavior_group.add(&catchup_row);

    // Upload concurrency (1–10 workers)
    let concurrency_adj = gtk::Adjustment::new(3.0, 1.0, 10.0, 1.0, 1.0, 0.0);
    let concurrency_row = adw::SpinRow::builder()
        .title("Upload Workers")
        .subtitle("Number of parallel upload workers (1–10). More workers = faster batch uploads.")
        .adjustment(&concurrency_adj)
        .build();
    behavior_group.add(&concurrency_row);

    // Quiet hours — enable switch + two hour spinners
    let quiet_hours_row = adw::SwitchRow::builder()
        .title("Quiet Hours")
        .subtitle("Pause uploads during a nightly window using your local clock.")
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

    // Show the hour spinners only when quiet hours are enabled
    quiet_hours_row.connect_active_notify(clone!(
        #[weak]
        quiet_start_row,
        #[weak]
        quiet_end_row,
        move |row| {
            quiet_start_row.set_sensitive(row.is_active());
            quiet_end_row.set_sensitive(row.is_active());
        }
    ));

    // --- WATCH FOLDERS GROUP ---
    let folders_group = adw::PreferencesGroup::builder()
        .title("Watch Folders")
        .description("Add folders with the picker so Mimick can keep access to them.")
        .build();
    settings_page.add(&folders_group);

    let startup_initial = config.data.run_on_startup;
    let tracked_rows = Rc::new(RefCell::new(Vec::<FolderRowData>::new()));
    let albums: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));

    // Reuse the application-wide API client — do NOT create a new one here.
    // Creating a new reqwest Client per window open allocates a new connection pool
    // that takes ~30s to self-clean, causing RAM to grow with each open/close cycle.
    let albums_ref = albums.clone();

    if let Some(client) = api_client.clone() {
        // Downgrade the window to a weak ref BEFORE the spawn.
        // After the async await, we upgrade it — if it's None the window was closed
        // while the API call was in-flight. We bail immediately, releasing all strong
        // refs to FolderRowData (and their contained GTK widgets) so they can be freed.
        // Without this, rapid open/close cycles would accumulate orphaned widget sets.
        let weak_win = window.downgrade();

        glib::MainContext::default().spawn_local(async move {
            let fetched = client.get_all_albums().await.unwrap_or_default();

            // Window may have been closed while we awaited the network response.
            // Bail out early — drops tracked_rows_async and albums_ref immediately.
            if weak_win.upgrade().is_none() {
                log::debug!("Settings window closed during album fetch — discarding result.");
                return;
            }

            *albums_ref.borrow_mut() = fetched.clone();

            // Album picker dialog fetches directly from albums_ref when opened.
            // We don't need to push updates to existing rows anymore.
        });
    }

    // List FIRST (matching Python layout), then Add button below
    let folders_list = ListBox::builder()
        .margin_top(12)
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    folders_group.add(&folders_list);

    let add_folder_btn = Button::builder().label("Add Folder").margin_top(12).build();
    folders_group.add(&add_folder_btn);

    // Add existing paths to listbox with album dropdown
    for entry in &config.data.watch_paths {
        #[allow(deprecated)]
        add_folder_row(&folders_list, entry, albums.clone(), &tracked_rows);
    }

    let folders_list_clone = folders_list.clone();
    let window_clone = window.clone();
    let tracked_rows_clone = tracked_rows.clone();
    let albums_clone = albums.clone();

    add_folder_btn.connect_clicked(move |_| {
        let dialog = FileDialog::builder().title("Select Watch Folder").build();
        let list_clone = folders_list_clone.clone();
        let tracked_clone = tracked_rows_clone.clone();
        let albums_ref = albums_clone.clone();

        dialog.select_folder(
            Some(&window_clone),
            gtk::gio::Cancellable::NONE,
            move |res| {
                if let Ok(file) = res
                    && let Some(path) = file.path()
                {
                    let path_str = path.to_string_lossy().to_string();
                    if tracked_clone.borrow().iter().any(|r| r.path == path_str) {
                        return;
                    }
                    #[allow(deprecated)]
                    add_folder_row(
                        &list_clone,
                        &WatchPathEntry::Simple(path_str),
                        albums_ref.clone(),
                        &tracked_clone,
                    );
                }
            },
        );
    });

    let controls_group = adw::PreferencesGroup::builder().title("Actions").build();
    status_page.add(&controls_group);

    // FlowBox so buttons wrap automatically on narrow widths
    let actions_flow = gtk::FlowBox::builder()
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(4)
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    controls_group.add(&actions_flow);

    let sync_now_btn = Button::builder()
        .label("Sync Now")
        .css_classes(vec!["suggested-action".to_string()])
        .hexpand(true)
        .build();
    actions_flow.insert(&sync_now_btn, -1);

    let pause_btn = Button::builder().label("Pause").hexpand(true).build();
    actions_flow.insert(&pause_btn, -1);

    let queue_btn = Button::builder()
        .label("Queue Inspector")
        .hexpand(true)
        .build();
    actions_flow.insert(&queue_btn, -1);

    let export_btn = Button::builder()
        .label("Export Diagnostics")
        .hexpand(true)
        .build();
    actions_flow.insert(&export_btn, -1);

    let app_group = adw::PreferencesGroup::builder()
        .title("Application")
        .build();
    settings_page.add(&app_group);

    let app_flow = gtk::FlowBox::builder()
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(3)
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    app_group.add(&app_flow);

    let about_btn = Button::builder()
        .label("About Mimick")
        .hexpand(true)
        .build();
    app_flow.insert(&about_btn, -1);

    let quit_btn = Button::builder()
        .label("Quit")
        .css_classes(vec!["destructive-action".to_string()])
        .hexpand(true)
        .build();
    app_flow.insert(&quit_btn, -1);

    let save_btn = Button::builder()
        .label("Save Changes")
        .css_classes(vec!["suggested-action".to_string()])
        .hexpand(true)
        .build();
    app_flow.insert(&save_btn, -1);

    let window_clone_about = window.clone();
    about_btn.connect_clicked(move |_| {
        show_about_dialog(&window_clone_about);
    });

    let update_save_btn_state = clone!(
        #[weak]
        api_key_entry,
        #[weak]
        save_btn,
        move || {
            let has_key = !api_key_entry.text().is_empty();
            save_btn.set_sensitive(has_key);

            if has_key {
                save_btn.add_css_class("suggested-action");
            } else {
                save_btn.remove_css_class("suggested-action");
            }
        }
    );
    update_save_btn_state();
    api_key_entry.connect_changed(move |_| {
        update_save_btn_state();
    });

    if let Some(qm) = queue_manager.clone() {
        let qm_for_inspector = qm.clone();
        queue_btn.connect_clicked(clone!(
            #[weak]
            window,
            move |_| {
                show_queue_inspector(&window, qm_for_inspector.clone());
            }
        ));

        let qm_for_pause = qm.clone();
        pause_btn.connect_clicked(clone!(
            #[weak]
            pause_btn,
            move |_| {
                let paused = !qm_for_pause.is_paused();
                qm_for_pause.set_paused(paused, paused.then(|| "Paused by user".to_string()));
                pause_btn.set_label(if paused { "Resume" } else { "Pause" });
            }
        ));
    } else {
        queue_btn.set_sensitive(false);
        pause_btn.set_sensitive(false);
    }

    if let Some(sync_now_tx) = sync_now_tx.clone() {
        sync_now_btn.connect_clicked(move |_| {
            let _ = sync_now_tx.send(());
        });
    } else {
        sync_now_btn.set_sensitive(false);
    }

    export_btn.connect_clicked(clone!(
        #[weak]
        window,
        #[strong]
        shared_state,
        move |_| {
            let dialog = FileDialog::builder()
                .title("Choose Diagnostics Export Folder")
                .build();
            let state = shared_state.clone();
            dialog.select_folder(
                Some(&window),
                gtk::gio::Cancellable::NONE,
                clone!(
                    #[weak]
                    window,
                    move |res| {
                        if let Ok(folder) = res
                            && let Some(path) = folder.path()
                        {
                            let state_snapshot = state.lock().unwrap().clone();
                            glib::MainContext::default().spawn_local(clone!(
                                #[weak]
                                window,
                                async move {
                                    let export_result = tokio::task::spawn_blocking(move || {
                                        diagnostics::export_bundle(&path, &state_snapshot)
                                    })
                                    .await;

                                    let (heading, body) = match export_result {
                                        Ok(Ok(bundle_dir)) => (
                                            "Diagnostics Exported",
                                            format!(
                                                "Saved diagnostics bundle to {}",
                                                bundle_dir.display()
                                            ),
                                        ),
                                        Ok(Err(err)) => (
                                            "Diagnostics Export Failed",
                                            format!("Could not write diagnostics bundle: {}", err),
                                        ),
                                        Err(err) => (
                                            "Diagnostics Export Failed",
                                            format!("Diagnostics task could not complete: {}", err),
                                        ),
                                    };

                                    let dialog = adw::MessageDialog::builder()
                                        .transient_for(&window)
                                        .heading(heading)
                                        .body(&body)
                                        .build();
                                    dialog.add_response("ok", "OK");
                                    dialog.present();
                                }
                            ));
                        }
                    }
                ),
            );
        }
    ));

    quit_btn.connect_clicked(clone!(
        #[strong]
        app_clone,
        move |_| {
            app_clone.quit();
        }
    ));

    save_btn.connect_clicked(clone!(
        #[weak]
        window,
        #[weak]
        internal_switch,
        #[weak]
        external_switch,
        #[weak]
        internal_entry,
        #[weak]
        external_entry,
        #[weak]
        api_key_entry,
        #[weak]
        startup_row,
        #[weak]
        metered_row,
        #[weak]
        battery_row,
        #[weak]
        notifications_row,
        #[weak]
        concurrency_row,
        #[weak]
        quiet_hours_row,
        #[weak]
        quiet_start_row,
        #[weak]
        quiet_end_row,
        #[weak]
        catchup_row,
        #[weak]
        save_btn,
        #[strong]
        tracked_rows,
        #[strong]
        albums,
        #[strong]
        shared_state,
        #[strong]
        api_client,
        #[strong]
        queue_manager,
        #[strong]
        monitor_handle,
        move |_| {
            save_btn.set_sensitive(false);

            let internal_url_enabled = internal_switch.is_active();
            let external_url_enabled = external_switch.is_active();
            let internal_url = internal_entry.text().to_string();
            let external_url = external_entry.text().to_string();
            let run_on_startup = startup_row.is_active();
            let pause_on_metered_network = metered_row.is_active();
            let pause_on_battery_power = battery_row.is_active();
            let notifications_enabled = notifications_row.is_active();
            let upload_concurrency = concurrency_row.value() as u8;
            let quiet_hours_enabled = quiet_hours_row.is_active();
            let quiet_hours_start = quiet_hours_enabled.then(|| quiet_start_row.value() as u8);
            let quiet_hours_end = quiet_hours_enabled.then(|| quiet_end_row.value() as u8);
            let catchup_mode = match catchup_row.selected() {
                1 => StartupCatchupMode::RecentOnly,
                2 => StartupCatchupMode::NewFilesOnly,
                _ => StartupCatchupMode::Full,
            };
            let mut watch_paths = Vec::new();
            let albums_map: HashMap<String, String> = albums.borrow().iter().cloned().collect();

            for row_data in tracked_rows.borrow().iter() {
                let folder = row_data.path.clone();
                let rules = row_data.rules.borrow().clone();
                let has_rules = rules != FolderRules::default();

                let album_name = row_data.album_name.borrow().clone();

                if (album_name.is_empty() || album_name == DEFAULT_ALBUM_LABEL) && !has_rules {
                    watch_paths.push(WatchPathEntry::Simple(folder));
                } else {
                    let album_id = albums_map.get(&album_name).cloned();
                    watch_paths.push(WatchPathEntry::WithConfig {
                        path: folder,
                        album_id,
                        album_name: if album_name.is_empty() || album_name == DEFAULT_ALBUM_LABEL {
                            None
                        } else {
                            Some(album_name)
                        },
                        rules,
                    });
                }
            }

            let api_key = api_key_entry.text().to_string();
            let runtime_internal_url = if internal_url_enabled {
                internal_url.clone()
            } else {
                String::new()
            };
            let runtime_external_url = if external_url_enabled {
                external_url.clone()
            } else {
                String::new()
            };
            let startup_changed = run_on_startup != startup_initial;

            glib::MainContext::default().spawn_local(clone!(
                #[weak]
                window,
                #[weak]
                startup_row,
                #[weak]
                save_btn,
                #[strong]
                shared_state,
                #[strong]
                api_client,
                #[strong]
                queue_manager,
                #[strong]
                monitor_handle,
                async move {
                    if startup_changed {
                        match autostart::apply(&window, run_on_startup).await {
                            Ok(granted) if granted == run_on_startup => {}
                            Ok(_) => {
                                startup_row.set_active(false);
                                save_btn.set_sensitive(true);

                                let dialog = adw::MessageDialog::builder()
                                    .transient_for(&window)
                                    .heading("Startup Permission Needed")
                                    .body("Mimick was not allowed to start automatically at login.")
                                    .build();
                                dialog.add_response("ok", "OK");
                                dialog.present();
                                return;
                            }
                            Err(err) => {
                                startup_row.set_active(startup_initial);
                                save_btn.set_sensitive(true);

                                let dialog = adw::MessageDialog::builder()
                                    .transient_for(&window)
                                    .heading("Could Not Update Startup Setting")
                                    .body(&err)
                                    .build();
                                dialog.add_response("ok", "OK");
                                dialog.present();
                                return;
                            }
                        }
                    }

                    let mut new_config = Config::new();
                    new_config.data.internal_url_enabled = internal_url_enabled;
                    new_config.data.external_url_enabled = external_url_enabled;
                    new_config.data.internal_url = internal_url;
                    new_config.data.external_url = external_url;
                    new_config.data.watch_paths = watch_paths.clone();
                    new_config.data.run_on_startup = run_on_startup;
                    new_config.data.pause_on_metered_network = pause_on_metered_network;
                    new_config.data.pause_on_battery_power = pause_on_battery_power;
                    new_config.data.notifications_enabled = notifications_enabled;
                    new_config.data.startup_catchup_mode = catchup_mode;
                    new_config.data.upload_concurrency = upload_concurrency;
                    new_config.data.quiet_hours_start = quiet_hours_start;
                    new_config.data.quiet_hours_end = quiet_hours_end;

                    if !api_key.is_empty() && !new_config.set_api_key(&api_key) {
                        save_btn.set_sensitive(true);

                        let dialog = adw::MessageDialog::builder()
                            .transient_for(&window)
                            .heading("Could Not Save API Key")
                            .body("Mimick could not store the API key in your desktop keyring.")
                            .build();
                        dialog.add_response("ok", "OK");
                        dialog.present();
                        return;
                    }

                    if !new_config.save() {
                        save_btn.set_sensitive(true);

                        let dialog = adw::MessageDialog::builder()
                            .transient_for(&window)
                            .heading("Could Not Save Settings")
                            .body("Mimick could not write the updated configuration to disk.")
                            .build();
                        dialog.add_response("ok", "OK");
                        dialog.present();
                        return;
                    }

                    if let Some(client) = api_client.clone() {
                        client
                            .update_settings(
                                runtime_internal_url.clone(),
                                runtime_external_url.clone(),
                                api_key.clone(),
                            )
                            .await;
                    }

                    if let Some(qm) = queue_manager.clone() {
                        qm.set_worker_limit(upload_concurrency);
                        qm.update_environment_policy(crate::queue_manager::EnvironmentPolicy {
                            pause_on_metered_network,
                            pause_on_battery_power,
                            quiet_hours_start,
                            quiet_hours_end,
                        });
                    }

                    crate::notifications::set_enabled(notifications_enabled);

                    if let Some(monitor) = monitor_handle.clone() {
                        monitor.replace_watch_paths(watch_paths.clone());
                    }

                    {
                        let mut state = shared_state.lock().unwrap();
                        state.watched_folder_count = watch_paths.len();
                        let current_paths = watch_paths
                            .iter()
                            .map(|entry| entry.path().to_string())
                            .collect::<std::collections::HashSet<_>>();
                        state
                            .folder_statuses
                            .retain(|path, _| current_paths.contains(path));
                    }

                    save_btn.set_sensitive(true);
                    save_btn.set_label("Saved");
                    save_btn.remove_css_class("suggested-action");

                    glib::timeout_add_local_once(
                        Duration::from_secs(2),
                        clone!(
                            #[weak]
                            save_btn,
                            move || {
                                save_btn.set_label("Save Changes");
                                save_btn.add_css_class("suggested-action");
                            }
                        ),
                    );
                }
            ));
        }
    ));

    // Populate from config
    internal_switch.set_active(config.data.internal_url_enabled);
    external_switch.set_active(config.data.external_url_enabled);
    internal_entry.set_text(&config.data.internal_url);
    external_entry.set_text(&config.data.external_url);
    internal_entry.set_sensitive(config.data.internal_url_enabled);
    external_entry.set_sensitive(config.data.external_url_enabled);
    startup_row.set_active(config.data.run_on_startup);
    metered_row.set_active(config.data.pause_on_metered_network);
    battery_row.set_active(config.data.pause_on_battery_power);
    notifications_row.set_active(config.data.notifications_enabled);
    concurrency_row.set_value(config.data.upload_concurrency as f64);
    let qh_enabled = config.data.quiet_hours_start.is_some();
    quiet_hours_row.set_active(qh_enabled);
    quiet_start_row.set_value(config.data.quiet_hours_start.unwrap_or(22) as f64);
    quiet_end_row.set_value(config.data.quiet_hours_end.unwrap_or(7) as f64);
    quiet_start_row.set_sensitive(qh_enabled);
    quiet_end_row.set_sensitive(qh_enabled);
    catchup_row.set_selected(match config.data.startup_catchup_mode {
        StartupCatchupMode::Full => 0,
        StartupCatchupMode::RecentOnly => 1,
        StartupCatchupMode::NewFilesOnly => 2,
    });

    if let Some(key) = config.get_api_key() {
        api_key_entry.set_text(&key);
    }

    // Toggle validation — at least one URL must always be enabled
    internal_switch.connect_active_notify(clone!(
        #[weak]
        external_switch,
        #[weak]
        internal_entry,
        #[weak]
        window,
        move |switch| {
            if !switch.is_active() && !external_switch.is_active() {
                switch.set_active(true);
                let dialog = adw::MessageDialog::builder()
                    .transient_for(&window)
                    .heading("Invalid Selection")
                    .body("At least one URL (Internal or External) must be enabled.")
                    .build();
                dialog.add_response("ok", "OK");
                dialog.present();
            }
            internal_entry.set_sensitive(switch.is_active());
        }
    ));

    external_switch.connect_active_notify(clone!(
        #[weak]
        internal_switch,
        #[weak]
        external_entry,
        #[weak]
        window,
        move |switch| {
            if !switch.is_active() && !internal_switch.is_active() {
                switch.set_active(true);
                let dialog = adw::MessageDialog::builder()
                    .transient_for(&window)
                    .heading("Invalid Selection")
                    .body("At least one URL (Internal or External) must be enabled.")
                    .build();
                dialog.add_response("ok", "OK");
                dialog.present();
            }
            external_entry.set_sensitive(switch.is_active());
        }
    ));

    // Background state poller — reads directly from in-memory shared state.
    // No disk I/O; the timer tears itself down automatically when the window closes
    // because the weak references to status_row / progress_bar fail to upgrade.
    glib::timeout_add_local(
        Duration::from_millis(500),
        clone!(
            #[weak]
            status_row,
            #[weak]
            progress_bar,
            #[weak]
            route_row,
            #[weak]
            folders_row,
            #[weak]
            queue_health_row,
            #[weak]
            last_sync_row,
            #[weak]
            error_row,
            #[weak]
            pause_btn,
            #[strong]
            tracked_rows,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move || {
                let (
                    status,
                    progress,
                    processed,
                    total,
                    failed,
                    current_file,
                    paused,
                    pause_reason,
                    pending,
                    route,
                    watched_folder_count,
                    last_successful_sync_at,
                    last_error,
                    last_error_guidance,
                    folder_statuses,
                ) = {
                    let s = shared_state.lock().unwrap();
                    (
                        s.status.clone(),
                        s.progress,
                        s.processed_count,
                        s.total_queued,
                        s.failed_count,
                        s.current_file.clone().unwrap_or_else(|| "...".to_string()),
                        s.paused,
                        s.pause_reason.clone(),
                        s.queue_size,
                        s.active_server_route.clone(),
                        s.watched_folder_count,
                        s.last_successful_sync_at,
                        s.last_error.clone(),
                        s.last_error_guidance.clone(),
                        s.folder_statuses.clone(),
                    )
                }; // lock released here

                pause_btn.set_label(if paused { "Resume" } else { "Pause" });
                route_row.set_subtitle(
                    route
                        .as_deref()
                        .map(|route| match route {
                            "LAN" => "Connected through LAN",
                            "WAN" => "Connected through WAN",
                            _ => "Connected through configured server",
                        })
                        .unwrap_or("Waiting for a successful connection check"),
                );
                folders_row.set_subtitle(&format!("{} configured", watched_folder_count));
                queue_health_row
                    .set_subtitle(&format!("{} pending, {} waiting to retry", pending, failed));
                last_sync_row.set_subtitle(&format_sync_age(last_successful_sync_at));
                error_row.set_title(last_error.as_deref().unwrap_or("No recent errors"));
                error_row.set_subtitle(
                    last_error_guidance
                        .as_deref()
                        .unwrap_or("Uploads are healthy."),
                );
                for row_data in tracked_rows.borrow().iter() {
                    let path = &row_data.path;
                    let mut final_subtitle = row_data.base_subtitle.clone();
                    if let Some(folder_status) = folder_statuses.get(path) {
                        if !final_subtitle.is_empty() {
                            final_subtitle.push('\n');
                        }
                        if let Some(err) = &folder_status.last_error {
                            final_subtitle.push_str(&format!("⚠️ Error: {}", err));
                        } else {
                            let mut txt = format!("Pending: {}", folder_status.pending_count);
                            if let Some(t) = folder_status.last_sync_at {
                                txt.push_str(&format!(
                                    " • Last Sync: {}",
                                    format_sync_age(Some(t))
                                ));
                            }
                            final_subtitle.push_str(&txt);
                        }
                    } else {
                        if !final_subtitle.is_empty() {
                            final_subtitle.push('\n');
                        }
                        final_subtitle.push_str("Status: Idle");
                    }
                    row_data.action_row.set_subtitle(&final_subtitle);
                }

                if status == "paused" || paused {
                    status_row.set_title("Paused");
                    status_row.set_subtitle(
                        pause_reason
                            .as_deref()
                            .unwrap_or("Sync has been temporarily paused."),
                    );
                    progress_bar.set_fraction((progress as f64) / 100.0);
                } else if status == "idle" {
                    if failed > 0 {
                        status_row.set_title("Offline / Waiting");
                        status_row.set_subtitle(&format!("{} item(s) pending network", failed));
                        progress_bar.set_fraction(1.0);
                    } else {
                        status_row.set_title("Idle");
                        status_row.set_subtitle(&format!(
                            "Successfully processed {} file(s)",
                            processed.saturating_sub(failed)
                        ));
                        progress_bar.set_fraction(if processed > 0 { 1.0 } else { 0.0 });
                    }
                } else if status == "uploading" {
                    let filename = std::path::Path::new(&current_file)
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| std::borrow::Cow::Borrowed("..."));
                    status_row.set_title(&format!("Uploading ({}/{})", processed, total));
                    status_row.set_subtitle(&filename);
                    progress_bar.set_fraction((progress as f64) / 100.0);
                }

                glib::ControlFlow::Continue
            }
        ),
    );
    // Hide instead of destroy on close.
    // The GTK widget tree (CSS caches, accessibility nodes, GSlice pools, GL state)
    // is built once and reused on every open/close cycle — zero new allocations per open.
    // open_settings_if_needed calls win.present() on the hidden window, which is
    // guaranteed to be in app.windows() even when not visible.
    window.connect_close_request(|win| {
        win.set_visible(false);
        glib::Propagation::Stop // prevent the default destroy
    });

    window.present();
}

fn add_folder_row(
    list: &ListBox,
    entry: &WatchPathEntry,
    albums_ref: Rc<RefCell<Vec<(String, String)>>>,
    tracked_rows: &Rc<RefCell<Vec<FolderRowData>>>,
) {
    let path = entry.path().to_string();
    let base_subtitle = watch_path_subtitle(&path).unwrap_or_default().to_string();

    let mut initial_subtitle = base_subtitle.clone();
    if !initial_subtitle.is_empty() {
        initial_subtitle.push('\n');
    }
    initial_subtitle.push_str("Status: Idle");

    let expander_row = adw::ExpanderRow::builder()
        .title(display_watch_path(&path))
        .subtitle(&initial_subtitle)
        .subtitle_lines(2)
        .title_lines(1)
        .build();

    let album_name = Rc::new(RefCell::new(
        entry
            .album_name()
            .unwrap_or(DEFAULT_ALBUM_LABEL)
            .to_string(),
    ));

    let rules = Rc::new(RefCell::new(entry.rules()));

    let picker_btn = Button::builder()
        .label(format!("Album: {}", album_name.borrow()))
        .valign(gtk::Align::Center)
        .tooltip_text("Select or create an target Immich album")
        .build();

    let picker_btn_clone = picker_btn.clone();
    let album_name_clone = album_name.clone();
    let albums_ref_clone = albums_ref.clone();

    picker_btn.connect_clicked(clone!(
        #[weak]
        expander_row,
        move |_| {
            if let Some(window) = expander_row
                .root()
                .and_then(|root| root.downcast::<adw::PreferencesWindow>().ok())
            {
                let window_clone = window.clone();
                let albums_ref_clone = albums_ref_clone.clone();
                let album_name_clone = album_name_clone.clone();
                let picker_btn_clone = picker_btn_clone.clone();

                glib::idle_add_local_once(move || {
                    show_album_picker_dialog(
                        &window_clone,
                        albums_ref_clone,
                        album_name_clone,
                        picker_btn_clone,
                    );
                });
            }
        }
    ));

    let album_subrow = adw::ActionRow::builder().title("Target Album").build();
    album_subrow.add_suffix(&picker_btn);
    expander_row.add_row(&album_subrow);

    let remove_btn = Button::builder()
        .icon_name("user-trash-symbolic")
        .valign(gtk::Align::Center)
        .css_classes(vec!["destructive-action".to_string()])
        .build();
    let rules_btn = Button::builder()
        .label("Rules")
        .tooltip_text("Edit folder rules")
        .valign(gtk::Align::Center)
        .build();

    let list_clone = list.clone();
    let tracked_clone = tracked_rows.clone();
    let path_clone = path.clone();
    let rules_clone = rules.clone();
    let path_for_rules = path.clone();

    rules_btn.connect_clicked(clone!(
        #[weak]
        expander_row,
        move |_| {
            if let Some(window) = expander_row
                .root()
                .and_then(|root| root.downcast::<adw::PreferencesWindow>().ok())
            {
                let window = window.clone();
                let path_for_rules = path_for_rules.clone();
                let rules_clone = rules_clone.clone();
                glib::idle_add_local_once(move || {
                    show_folder_rules_dialog(&window, &path_for_rules, rules_clone);
                });
            }
        }
    ));

    remove_btn.connect_clicked(clone!(
        #[weak]
        expander_row,
        move |_| {
            let list_clone = list_clone.clone();
            let tracked_clone = tracked_clone.clone();
            let path_clone = path_clone.clone();
            let expander_row = expander_row.clone();
            glib::idle_add_local_once(move || {
                if let Some(focus_target) = list_clone.first_child() {
                    focus_target.grab_focus();
                }
                list_clone.remove(&expander_row);
                tracked_clone.borrow_mut().retain(|r| r.path != path_clone);
            });
        }
    ));

    let rules_subrow = adw::ActionRow::builder().title("Folder Rules").build();
    rules_subrow.add_suffix(&rules_btn);
    expander_row.add_row(&rules_subrow);

    let remove_subrow = adw::ActionRow::builder().title("Remove Folder").build();
    remove_subrow.add_suffix(&remove_btn);
    expander_row.add_row(&remove_subrow);

    list.append(&expander_row);
    tracked_rows.borrow_mut().push(FolderRowData {
        path,
        album_name,
        rules,
        action_row: expander_row,
        base_subtitle,
    });
}

fn show_folder_rules_dialog(
    parent: &impl gtk::prelude::IsA<gtk::Window>,
    folder_path: &str,
    rules_state: Rc<RefCell<FolderRules>>,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Folder Rules")
        .default_width(420)
        .build();
    let content = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    dialog.set_content(Some(&content));

    let title = gtk::Label::builder()
        .label(format!("Rules for {}", display_watch_path(folder_path)))
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    content.append(&title);

    let current = rules_state.borrow().clone();

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec![String::from("boxed-list")])
        .build();

    let ignore_hidden = adw::SwitchRow::builder()
        .title("Ignore Hidden Files / Folders")
        .subtitle("Skip paths that contain hidden components such as .cache or .thumbnails.")
        .active(current.ignore_hidden)
        .build();

    list_box.append(&ignore_hidden);
    content.append(&list_box);

    let max_size_entry = Entry::builder()
        .placeholder_text("Max file size in MB, leave blank for no limit")
        .width_request(0)
        .text(
            current
                .max_file_size_mb
                .map(|value| value.to_string())
                .unwrap_or_default(),
        )
        .build();
    content.append(&max_size_entry);

    let extensions_entry = Entry::builder()
        .placeholder_text("Comma-separated extensions, e.g. jpg,png,mp4")
        .width_request(0)
        .text(current.allowed_extensions.join(", "))
        .build();
    content.append(&extensions_entry);

    let actions = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel_btn = Button::builder().label("Cancel").build();
    let save_btn = Button::builder()
        .label("Save")
        .css_classes(vec!["suggested-action".to_string()])
        .build();
    actions.append(&cancel_btn);
    actions.append(&save_btn);
    content.append(&actions);

    cancel_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    save_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            let max_file_size_mb = max_size_entry.text().trim().parse::<u64>().ok();
            let allowed_extensions = extensions_entry
                .text()
                .split(',')
                .map(|part| part.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();

            *rules_state.borrow_mut() = FolderRules {
                ignore_hidden: ignore_hidden.is_active(),
                max_file_size_mb,
                allowed_extensions,
            };
            dialog.close();
        }
    ));

    dialog.present();
}

fn show_queue_inspector(
    parent: &impl gtk::prelude::IsA<gtk::Window>,
    queue_manager: Arc<QueueManager>,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Queue Inspector")
        .default_width(760)
        .default_height(560)
        .build();
    let content = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    dialog.set_content(Some(&content));

    let actions = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    content.append(&actions);

    let retry_all_btn = Button::builder().label("Retry All Failed").build();
    let clear_failed_btn = Button::builder().label("Clear Failed Queue").build();
    actions.append(&retry_all_btn);
    actions.append(&clear_failed_btn);

    let failed_group = adw::PreferencesGroup::builder()
        .title("Failed Retry Queue")
        .build();
    content.append(&failed_group);

    let failed_tasks = queue_manager.failed_tasks();
    if failed_tasks.is_empty() {
        failed_group.add(
            &adw::ActionRow::builder()
                .title("No failed items")
                .subtitle("The retry queue is currently empty.")
                .build(),
        );
    } else {
        for task in failed_tasks {
            let row = adw::ActionRow::builder()
                .title(
                    Path::new(&task.path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(task.path.as_str()),
                )
                .subtitle(&task.path)
                .build();
            let retry_btn = Button::builder().label("Retry").build();
            let task_path = task.path.clone();
            let qm = queue_manager.clone();
            retry_btn.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                let qm = qm.clone();
                let task_path = task_path.clone();
                glib::MainContext::default().spawn_local(async move {
                    let _ = qm.retry_failed_path(&task_path).await;
                });
            });
            row.add_suffix(&retry_btn);
            failed_group.add(&row);
        }
    }

    let events_group = adw::PreferencesGroup::builder()
        .title("Recent Queue Activity")
        .build();
    content.append(&events_group);

    let events_scroll = ScrolledWindow::builder()
        .min_content_height(280)
        .vexpand(true)
        .build();
    let events_list = ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    events_scroll.set_child(Some(&events_list));
    events_group.add(&events_scroll);

    for event in queue_manager.recent_events() {
        let row = adw::ActionRow::builder()
            .title(
                Path::new(&event.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(event.path.as_str()),
            )
            .subtitle(format!(
                "{} | attempts={}{}",
                event.status,
                event.attempts,
                event
                    .detail
                    .as_ref()
                    .map(|detail| format!(" | {}", detail))
                    .unwrap_or_default()
            ))
            .build();
        row.add_prefix(
            &gtk::Label::builder()
                .label(display_watch_path(&event.path))
                .wrap(true)
                .halign(gtk::Align::Start)
                .build(),
        );
        events_list.append(&row);
    }

    let qm_retry_all = queue_manager.clone();
    retry_all_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        let qm = qm_retry_all.clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = qm.retry_all_failed().await;
        });
    });

    let qm_clear = queue_manager.clone();
    clear_failed_btn.connect_clicked(move |_| {
        let _ = qm_clear.clear_failed();
    });

    let close_btn = Button::builder().label("Close").build();
    close_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));
    content.append(&close_btn);
    dialog.present();
}

fn show_about_dialog(parent: &impl gtk::prelude::IsA<gtk::Window>) {
    // Register asset search path so the "icon" name resolves
    let display = gtk::gdk::Display::default();
    if let Some(display) = display {
        let theme = gtk::IconTheme::for_display(&display);
        let assets_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets");
        theme.add_search_path(&assets_dir);
    }

    let about = adw::AboutWindow::builder()
        .application_name("Mimick")
        .application_icon("io.github.nicx17.mimick")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Nick Cardoso")
        .website("https://github.com/nicx17/mimick")
        .issue_url("https://github.com/nicx17/mimick/issues")
        .license_type(gtk::License::Gpl30)
        .transient_for(parent)
        .build();

    about.add_credit_section(
        Some("Icon Design"),
        &["Round Icons https://unsplash.com/illustrations/a-white-and-orange-flower-on-a-white-background-IkQ_WrJzZOM"],
    );

    about.present();
}

fn show_album_picker_dialog(
    parent: &impl gtk::prelude::IsA<gtk::Window>,
    albums_ref: Rc<RefCell<Vec<(String, String)>>>,
    target_album_state: Rc<RefCell<String>>,
    trigger_btn: Button,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Select Album")
        .default_width(400)
        .default_height(500)
        .build();

    let header_bar = adw::HeaderBar::new();
    let vbox = Box::builder().orientation(Orientation::Vertical).build();
    dialog.set_content(Some(&vbox));
    vbox.append(&header_bar);

    let search_entry = gtk::SearchEntry::builder()
        .halign(gtk::Align::Center)
        .width_request(300)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    vbox.append(&search_entry);

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();
    list_box.add_css_class("boxed-list");

    let scrolled_window = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    scrolled_window.set_child(Some(&list_box));
    vbox.append(&scrolled_window);

    // Dynamic filtering capability
    let albums_ref_cloned = albums_ref.clone();
    let apply_filter = {
        let list_box = list_box.clone();
        let dialog = dialog.clone();
        let target_album_state = target_album_state.clone();
        let trigger_btn = trigger_btn.clone();

        move |query: &str| {
            // Clear existing
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let q = query.trim().to_lowercase();

            // Row 1: Default Folder Name (only if it matches search)
            if q.is_empty() || DEFAULT_ALBUM_LABEL.to_lowercase().contains(&q) {
                let default_row = adw::ActionRow::builder()
                    .title(DEFAULT_ALBUM_LABEL)
                    .subtitle("Creates album dynamically per-folder")
                    .activatable(true)
                    .build();
                let dialog_clone = dialog.clone();
                let state_clone = target_album_state.clone();
                let btn_clone = trigger_btn.clone();
                default_row.connect_activated(move |_| {
                    *state_clone.borrow_mut() = DEFAULT_ALBUM_LABEL.to_string();
                    btn_clone.set_label(&format!("Album: {}", DEFAULT_ALBUM_LABEL));
                    dialog_clone.close();
                });
                list_box.append(&default_row);
            }

            // Row 2: Create Custom (if query is typed)
            if !q.is_empty() {
                let typed_raw = query.trim().to_string();
                let create_row = adw::ActionRow::builder()
                    .title(format!("Create new: \"{}\"", typed_raw))
                    .activatable(true)
                    .build();
                let dialog_clone = dialog.clone();
                let state_clone = target_album_state.clone();
                let btn_clone = trigger_btn.clone();
                create_row.connect_activated(move |_| {
                    *state_clone.borrow_mut() = typed_raw.clone();
                    btn_clone.set_label(&format!("Album: {}", typed_raw));
                    dialog_clone.close();
                });
                list_box.append(&create_row);
            }

            // Row 3+: Remote Albums
            for (name, _) in albums_ref_cloned.borrow().iter() {
                if name == DEFAULT_ALBUM_LABEL {
                    continue; // Skip the "Use default folder name" if we pushed it above
                }
                if q.is_empty() || name.to_lowercase().contains(&q) {
                    let album_name = name.clone();
                    let row = adw::ActionRow::builder()
                        .title(&album_name)
                        .activatable(true)
                        .build();
                    let dialog_clone = dialog.clone();
                    let state_clone = target_album_state.clone();
                    let btn_clone = trigger_btn.clone();
                    let album_name_clone = album_name.clone();
                    row.connect_activated(move |_| {
                        *state_clone.borrow_mut() = album_name_clone.clone();
                        btn_clone.set_label(&format!("Album: {}", album_name_clone));
                        dialog_clone.close();
                    });
                    list_box.append(&row);
                }
            }
        }
    };

    // Initial populate
    apply_filter("");

    let apply_filter_rc = Rc::new(apply_filter);
    search_entry.connect_search_changed(move |entry| {
        apply_filter_rc(&entry.text());
    });

    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::format_sync_age;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_format_sync_age_for_missing_timestamp() {
        assert_eq!(format_sync_age(None), "No successful sync yet");
    }

    #[test]
    fn test_format_sync_age_for_recent_timestamp() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        assert_eq!(format_sync_age(Some(now - 30.0)), "Less than a minute ago");
    }
}
