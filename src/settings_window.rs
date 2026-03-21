//! GTK4/Libadwaita settings window and status dashboard.

use crate::autostart;
use crate::config::{FolderRules, WatchPathEntry};
use crate::diagnostics;
use adw::prelude::*;
use glib::clone;
use gtk::prelude::*;
use gtk::{
    Box, Button, DropDown, Entry, FileDialog, ListBox, Orientation, PasswordEntry, ProgressBar,
    ScrolledWindow, Stack, StringList, Switch,
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
use crate::queue_manager::QueueManager;
use crate::restart::request_restart;
use crate::state_manager::AppState;
use crate::watch_path_display::{display_watch_path, watch_path_subtitle};

/// GTK widgets kept around for a single watch-folder row in the settings list.
struct FolderRowData {
    pub path: String,
    pub dropdown: DropDown,
    pub string_list: StringList,
    pub custom_entry: Entry,
    pub rules: Rc<RefCell<FolderRules>>,
}

/// Build the main settings window and wire it to the shared app state.
pub fn build_settings_window(
    app: &adw::Application,
    shared_state: Arc<Mutex<AppState>>,
    api_client: Option<Arc<ImmichApiClient>>,
    queue_manager: Option<Arc<QueueManager>>,
    sync_now_tx: Option<UnboundedSender<()>>,
) {
    // Use an ApplicationWindow so Libadwaita manages the titlebar and window lifecycle.
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Mimick Settings")
        .default_width(660)
        .default_height(840)
        .build();
    let app_clone = app.clone();

    // Force Dark Theme
    let style_mgr = adw::StyleManager::default();
    style_mgr.set_color_scheme(adw::ColorScheme::ForceDark);

    let vbox = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .build();
    window.set_content(Some(&vbox));

    // HeaderBar lives inside the content box – no double titlebar
    let header_bar = adw::HeaderBar::new();
    vbox.append(&header_bar);

    let page_stack = Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .build();
    let stack_switcher = gtk::StackSwitcher::new();
    stack_switcher.set_stack(Some(&page_stack));
    header_bar.set_title_widget(Some(&stack_switcher));

    // About Button
    let about_btn = Button::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text("About Mimick")
        .build();
    let window_clone = window.clone();
    about_btn.connect_clicked(move |_| {
        show_about_dialog(&window_clone);
    });
    header_bar.pack_start(&about_btn);

    let setup_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    let controls_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    page_stack.add_titled(&setup_scroll, Some("setup"), "Setup");
    page_stack.add_titled(&controls_scroll, Some("controls"), "Controls");
    page_stack.set_visible_child_name("setup");
    vbox.append(&page_stack);

    let setup_clamp = adw::Clamp::builder()
        .maximum_size(640)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    setup_scroll.set_child(Some(&setup_clamp));

    let controls_clamp = adw::Clamp::builder()
        .maximum_size(640)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    controls_scroll.set_child(Some(&controls_clamp));

    let setup_page_box = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(18)
        .build();
    setup_clamp.set_child(Some(&setup_page_box));

    let controls_page_box = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(18)
        .build();
    controls_clamp.set_child(Some(&controls_page_box));

    // --- PROGRESS GROUP ---
    let progress_group = adw::PreferencesGroup::builder()
        .title("Sync Status")
        .build();
    controls_page_box.append(&progress_group);

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

    // --- CONNECTIVITY GROUP ---
    let conn_group = adw::PreferencesGroup::builder()
        .title("Connectivity")
        .build();
    setup_page_box.append(&conn_group);

    // Internal URL
    let internal_row = adw::ActionRow::builder()
        .title("Internal URL (LAN)")
        .build();
    let internal_switch = Switch::builder().valign(gtk::Align::Center).build();
    let internal_entry = Entry::builder()
        .placeholder_text("http://192.168.1.10:2283")
        .valign(gtk::Align::Center)
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
    setup_page_box.append(&behavior_group);

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

    // --- WATCH FOLDERS GROUP ---
    let folders_group = adw::PreferencesGroup::builder()
        .title("Watch Folders")
        .description("Add folders with the picker so Mimick can keep access to them.")
        .build();
    setup_page_box.append(&folders_group);

    let config = Config::new();
    let startup_initial = config.data.run_on_startup;
    let tracked_rows = Rc::new(RefCell::new(Vec::<FolderRowData>::new()));
    let albums: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));

    // Reuse the application-wide API client — do NOT create a new one here.
    // Creating a new reqwest Client per window open allocates a new connection pool
    // that takes ~30s to self-clean, causing RAM to grow with each open/close cycle.
    let albums_ref = albums.clone();
    let tracked_rows_async = tracked_rows.clone();

    if let Some(client) = api_client {
        // Downgrade the window to a weak ref BEFORE the spawn.
        // After the async await, we upgrade it — if it's None the window was closed
        // while the API call was in-flight. We bail immediately, releasing all strong
        // refs to FolderRowData (and their contained GTK widgets) so they can be freed.
        // Without this, rapid open/close cycles would accumulate orphaned widget sets.
        let weak_win = window.downgrade();

        glib::MainContext::default().spawn_local(async move {
            let fetched = client.get_all_albums().await;

            // Window may have been closed while we awaited the network response.
            // Bail out early — drops tracked_rows_async and albums_ref immediately.
            if weak_win.upgrade().is_none() {
                log::debug!("Settings window closed during album fetch — discarding result.");
                return;
            }

            *albums_ref.borrow_mut() = fetched.clone();

            for row_data in tracked_rows_async.borrow().iter() {
                let current_selected = row_data.dropdown.selected();
                let mut current_text = None;
                if current_selected < row_data.string_list.n_items()
                    && let Some(s) = row_data.string_list.string(current_selected)
                {
                    current_text = Some(s.to_string());
                }

                row_data.string_list.splice(
                    0,
                    row_data.string_list.n_items(),
                    &["Default (Folder Name)"],
                );
                for (name, _) in &fetched {
                    if name != "Default (Folder Name)" {
                        row_data.string_list.append(name);
                    }
                }
                row_data.string_list.append("Custom Album...");

                if let Some(text) = current_text {
                    let mut found = false;
                    for i in 0..row_data.string_list.n_items() {
                        if let Some(s) = row_data.string_list.string(i)
                            && s.as_str() == text
                        {
                            row_data.dropdown.set_selected(i);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        if text == "Custom Album..." {
                            row_data
                                .dropdown
                                .set_selected(row_data.string_list.n_items() - 1);
                        } else {
                            row_data.dropdown.set_selected(0);
                        }
                    }
                }
            }
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
        add_folder_row(&folders_list, entry, &albums.borrow(), &tracked_rows);
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
                        &albums_ref.borrow(),
                        &tracked_clone,
                    );
                }
            },
        );
    });

    let controls_group = adw::PreferencesGroup::builder().title("Actions").build();
    controls_page_box.append(&controls_group);

    let controls_box = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    controls_group.add(&controls_box);

    let primary_actions_row = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    controls_box.append(&primary_actions_row);

    let secondary_actions_row = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    controls_box.append(&secondary_actions_row);

    let sync_now_btn = Button::builder()
        .label("Sync Now")
        .css_classes(vec!["suggested-action".to_string()])
        .hexpand(true)
        .build();
    primary_actions_row.append(&sync_now_btn);

    let pause_btn = Button::builder().label("Pause").hexpand(true).build();
    primary_actions_row.append(&pause_btn);

    let queue_btn = Button::builder()
        .label("Queue Inspector")
        .hexpand(true)
        .build();
    secondary_actions_row.append(&queue_btn);

    let export_btn = Button::builder()
        .label("Export Diagnostics")
        .hexpand(true)
        .build();
    secondary_actions_row.append(&export_btn);

    let footer_separator = gtk::Separator::new(Orientation::Horizontal);
    vbox.append(&footer_separator);

    let footer_box = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    let footer_spacer = Box::builder().hexpand(true).build();
    footer_box.append(&footer_spacer);

    let close_btn = Button::builder().label("Close").build();
    footer_box.append(&close_btn);

    let quit_btn = Button::builder()
        .label("Quit")
        .css_classes(vec!["destructive-action".to_string()])
        .build();
    footer_box.append(&quit_btn);

    let save_btn = Button::builder()
        .label("Save & Restart")
        .css_classes(vec!["suggested-action".to_string()])
        .build();
    footer_box.append(&save_btn);
    vbox.append(&footer_box);

    close_btn.connect_clicked(clone!(
        #[weak]
        window,
        move |_| {
            window.set_visible(false);
        }
    ));

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
        save_btn,
        #[strong]
        app_clone,
        #[strong]
        tracked_rows,
        #[strong]
        albums,
        move |_| {
            save_btn.set_sensitive(false);

            let internal_url_enabled = internal_switch.is_active();
            let external_url_enabled = external_switch.is_active();
            let internal_url = internal_entry.text().to_string();
            let external_url = external_entry.text().to_string();
            let run_on_startup = startup_row.is_active();
            let pause_on_metered_network = metered_row.is_active();
            let pause_on_battery_power = battery_row.is_active();
            let mut watch_paths = Vec::new();
            let albums_map: HashMap<String, String> = albums.borrow().iter().cloned().collect();

            for row_data in tracked_rows.borrow().iter() {
                let folder = row_data.path.clone();
                let selected_idx = row_data.dropdown.selected();
                let rules = row_data.rules.borrow().clone();
                let has_rules = rules != FolderRules::default();

                let album_name = if selected_idx == row_data.string_list.n_items() - 1 {
                    row_data.custom_entry.text().to_string()
                } else if let Some(s) = row_data.string_list.string(selected_idx) {
                    s.to_string()
                } else {
                    "Default (Folder Name)".to_string()
                };

                if (album_name.is_empty() || album_name == "Default (Folder Name)") && !has_rules {
                    watch_paths.push(WatchPathEntry::Simple(folder));
                } else {
                    let album_id = albums_map.get(&album_name).cloned();
                    watch_paths.push(WatchPathEntry::WithConfig {
                        path: folder,
                        album_id,
                        album_name: if album_name.is_empty()
                            || album_name == "Default (Folder Name)"
                        {
                            None
                        } else {
                            Some(album_name)
                        },
                        rules,
                    });
                }
            }

            let api_key = api_key_entry.text().to_string();
            let startup_changed = run_on_startup != startup_initial;

            glib::MainContext::default().spawn_local(clone!(
                #[weak]
                window,
                #[weak]
                startup_row,
                #[weak]
                save_btn,
                #[strong]
                app_clone,
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
                    new_config.data.watch_paths = watch_paths;
                    new_config.data.run_on_startup = run_on_startup;
                    new_config.data.pause_on_metered_network = pause_on_metered_network;
                    new_config.data.pause_on_battery_power = pause_on_battery_power;

                    if !api_key.is_empty() {
                        new_config.set_api_key(&api_key);
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

                    request_restart();
                    app_clone.quit();
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

    if let Some(key) = config.get_api_key() {
        api_key_entry.set_text(&key);
    }

    // Toggle validation – at least one URL must always be enabled
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
            pause_btn,
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
                    )
                }; // lock released here

                pause_btn.set_label(if paused { "Resume" } else { "Pause" });

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
    albums: &[(String, String)],
    tracked_rows: &Rc<RefCell<Vec<FolderRowData>>>,
) {
    let path = entry.path().to_string();
    let row = adw::ActionRow::builder()
        .title(display_watch_path(&path))
        .build();
    if let Some(subtitle) = watch_path_subtitle(&path) {
        row.set_subtitle(subtitle);
    }

    let string_list = gtk::StringList::new(&["Default (Folder Name)"]);
    for (name, _) in albums {
        if name != "Default (Folder Name)" {
            string_list.append(name);
        }
    }
    string_list.append("Custom Album...");

    let dropdown = gtk::DropDown::builder()
        .model(&string_list)
        .valign(gtk::Align::Center)
        .build();

    let custom_entry = gtk::Entry::builder()
        .placeholder_text("New album name")
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    let rules = Rc::new(RefCell::new(entry.rules()));

    if let Some(name) = entry.album_name()
        && name != "Default (Folder Name)"
    {
        let mut found = false;
        for i in 0..string_list.n_items() {
            if let Some(s) = string_list.string(i)
                && s.as_str() == name
            {
                dropdown.set_selected(i);
                found = true;
                break;
            }
        }
        if !found {
            dropdown.set_selected(string_list.n_items() - 1); // "Custom Album..."
            custom_entry.set_text(name);
            custom_entry.set_visible(true);
        }
    }

    let custom_entry_clone = custom_entry.clone();
    let string_list_clone = string_list.clone();
    dropdown.connect_selected_notify(move |dd| {
        let selected = dd.selected();
        if selected == string_list_clone.n_items() - 1 {
            custom_entry_clone.set_visible(true);
        } else {
            custom_entry_clone.set_visible(false);
        }
    });

    let suffix_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    suffix_box.append(&dropdown);
    suffix_box.append(&custom_entry);
    row.add_suffix(&suffix_box);

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
        row,
        move |_| {
            if let Some(window) = row
                .root()
                .and_then(|root| root.downcast::<adw::ApplicationWindow>().ok())
            {
                show_folder_rules_dialog(&window, &path_for_rules, rules_clone.clone());
            }
        }
    ));

    remove_btn.connect_clicked(clone!(
        #[weak]
        row,
        move |_| {
            list_clone.remove(&row);
            tracked_clone.borrow_mut().retain(|r| r.path != path_clone);
        }
    ));
    row.add_suffix(&rules_btn);
    row.add_suffix(&remove_btn);

    list.append(&row);
    tracked_rows.borrow_mut().push(FolderRowData {
        path,
        dropdown,
        string_list,
        custom_entry,
        rules,
    });
}

fn show_folder_rules_dialog(
    parent: &adw::ApplicationWindow,
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

    let ignore_hidden = adw::SwitchRow::builder()
        .title("Ignore Hidden Files / Folders")
        .subtitle("Skip paths that contain hidden components such as .cache or .thumbnails.")
        .active(current.ignore_hidden)
        .build();
    content.append(&ignore_hidden);

    let max_size_entry = Entry::builder()
        .placeholder_text("Max file size in MB, leave blank for no limit")
        .text(
            current
                .max_file_size_mb
                .map(|value| value.to_string())
                .unwrap_or_default(),
        )
        .build();
    content.append(&max_size_entry);

    let extensions_entry = Entry::builder()
        .placeholder_text("Allowed extensions, comma separated, leave blank for default media list")
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

fn show_queue_inspector(parent: &adw::ApplicationWindow, queue_manager: Arc<QueueManager>) {
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

fn show_about_dialog(parent: &adw::ApplicationWindow) {
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
