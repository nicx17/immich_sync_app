//! Implements the GTK4/Libadwaita settings window and status dashboard.
//!
//! Builds the tabbed preferences interface covering server configuration,
//! watch-folder management, behaviour toggles, and a live health dashboard.
//! Changes are validated and persisted to the JSON config file on save.

use crate::autostart;
use crate::config::{FolderRules, StartupCatchupMode, WatchPathEntry};
use crate::diagnostics;
use adw::prelude::*;
use glib::clone;
use gtk::prelude::*;
use gtk::{Button, FileDialog, ListBox, ScrolledWindow};
use libadwaita as adw;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::app_context::AppContext;

mod actions_ui;
mod behavior;
mod connectivity;
mod library;
mod queue_inspector;
mod status;
mod watch_folders;

use queue_inspector::show_about_dialog;
pub use queue_inspector::show_queue_inspector;
use watch_folders::add_folder_row;

/// Holds GTK widgets for a single watch-folder row in the settings list.
struct FolderRowData {
    /// Absolute local file path.
    path: String,
    /// Target Immich album name.
    album_name: Rc<RefCell<String>>,
    /// Custom path filtering rules.
    rules: Rc<RefCell<FolderRules>>,
    /// Libadwaita row widget representing this watched folder.
    action_row: adw::ExpanderRow,
    /// Base status subtitle for the row.
    base_subtitle: String,
}

const DEFAULT_ALBUM_LABEL: &str = "Default (Folder Name)";

/// Display a standard Libadwaita modal alert message dialog to the user.
fn show_alert(parent: &impl gtk::prelude::IsA<gtk::Widget>, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(Some(parent));
}

/// Format a Unix timestamp representing the last sync time into a relative display string.
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

/// Construct and present the settings window as a top-level window.
pub fn build_settings_window(app: &adw::Application, ctx: Arc<AppContext>) {
    build_settings_window_with_parent(app, ctx, None);
}

/// Construct and present the settings window, optionally transient for a parent window.
pub fn build_settings_window_with_parent(
    app: &adw::Application,
    ctx: Arc<AppContext>,
    parent: Option<&adw::ApplicationWindow>,
) {
    let shared_state = ctx.state.clone();
    let api_client = ctx.api_client.clone();
    let queue_manager = ctx.queue_manager.clone();
    let monitor_handle = ctx.monitor_handle.clone();
    let live_watch_paths = ctx.live_watch_paths.clone();
    let sync_now_tx = ctx.sync_now_tx.clone();
    let thumbnail_cache = ctx.thumbnail_cache.clone();
    let shared_config = ctx.config.clone();
    // Use an application window with a Libadwaita header switcher and two pages.
    let mut window_builder = adw::ApplicationWindow::builder()
        .application(app)
        .title("Mimick")
        .name("mimick-settings-window")
        .default_width(520)
        .default_height(780);
    if let Some(parent) = parent {
        window_builder = window_builder
            .transient_for(parent)
            .modal(true)
            .destroy_with_parent(true);
    }
    let window = window_builder.build();
    window.set_size_request(360, 480);

    let view_stack = adw::ViewStack::builder()
        .vexpand(true)
        .hexpand(true)
        .build();
    let page_switcher = adw::ViewSwitcher::builder().stack(&view_stack).build();
    let header_bar = adw::HeaderBar::builder()
        .title_widget(&page_switcher)
        .build();
    let about_header_btn = Button::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text("About Mimick")
        .build();
    let window_clone_about = window.clone();
    about_header_btn.connect_clicked(move |_| {
        show_about_dialog(&window_clone_about);
    });
    header_bar.pack_start(&about_header_btn);

    let toolbar_view = adw::ToolbarView::builder().build();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&view_stack));
    window.set_content(Some(&toolbar_view));

    let status_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();
    let settings_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();
    view_stack.add_titled_with_icon(
        &status_scroll,
        Some("status"),
        "Status",
        "dialog-information-symbolic",
    );
    view_stack.add_titled_with_icon(
        &settings_scroll,
        Some("settings"),
        "Settings",
        "emblem-system-symbolic",
    );

    let app_clone = app.clone();
    let config = ctx.config.read().clone();

    let status_page = adw::PreferencesPage::builder()
        .title("Status")
        .icon_name("dialog-information-symbolic")
        .build();
    status_scroll.set_child(Some(&status_page));

    let settings_page = adw::PreferencesPage::builder()
        .title("Settings")
        .icon_name("emblem-system-symbolic")
        .build();
    settings_scroll.set_child(Some(&settings_page));

    let is_unconfigured = config.get_api_key().unwrap_or_default().is_empty();
    if is_unconfigured {
        let welcome_group = adw::PreferencesGroup::builder()
            .title("Welcome to Mimick!")
            .description("Start by adding your API key, testing the connection, and choosing at least one folder. The key needs Asset (read, view, upload, update, download, delete), Album (read, create, update), and albumAsset (create, delete) permissions.")
            .build();

        let help_row = adw::ActionRow::builder()
            .title("How to get an API Key")
            .subtitle("Base sync: user.read, asset.upload/update, album.read/create, albumAsset.create. See docs for Library/deletion scopes.")
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

    let status::StatusWidgets {
        status_row,
        progress_bar,
        route_row,
        folders_row,
        queue_health_row,
        last_sync_row,
        error_row,
    } = status::build_status_group(&status_page);

    let connectivity::ConnectivityWidgets {
        internal_switch,
        external_switch,
        internal_entry,
        external_entry,
        api_key_entry,
        test_btn,
        save_btn,
    } = connectivity::build_connectivity_group(&settings_page, &window);

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
            let ping_client = api_client_for_test.clone();
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

                                show_alert(&window, heading, &report);

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

    let behavior::BehaviorWidgets {
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
    } = behavior::build_behavior_group(&settings_page);

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

    // --- LIBRARY GROUP ---
    let library::LibraryWidgets {
        library_group,
        preview_full_row,
        grid_quality_row,
        raw_full_decode_row,
        raw_cache_row,
        disk_cache_row,
    } = library::build_library_group(&settings_page);

    library_view_row.connect_active_notify(glib::clone!(
        #[weak]
        library_group,
        move |row| {
            library_group.set_visible(row.is_active());
        }
    ));

    let ctx_for_preview = ctx.clone();
    preview_full_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let mut cfg = ctx_for_preview.config.write();
        if cfg.data.library_preview_full_resolution != active {
            cfg.data.library_preview_full_resolution = active;
            cfg.save();
        }
    });

    let initial_quality_idx = match ctx.config.read().data.library_grid_quality.as_str() {
        "thumbnail" => 1,
        "preview" => 2,
        "fullsize" => 3,
        _ => 0,
    };
    grid_quality_row.set_selected(initial_quality_idx);
    let ctx_for_quality = ctx.clone();
    grid_quality_row.connect_selected_notify(move |row| {
        let value = match row.selected() {
            1 => "thumbnail",
            2 => "preview",
            3 => "fullsize",
            _ => "auto",
        };
        let mut cfg = ctx_for_quality.config.write();
        if cfg.data.library_grid_quality != value {
            cfg.data.library_grid_quality = value.to_string();
            cfg.save();
        }
    });

    raw_cache_row.set_sensitive(raw_full_decode_row.is_active());

    let cache_row_ref = raw_cache_row.clone();
    let ctx_for_raw_decode = ctx.clone();
    raw_full_decode_row.connect_active_notify(move |row| {
        let active = row.is_active();
        cache_row_ref.set_sensitive(active);
        crate::library::set_raw_full_decode(active);
        let mut cfg = ctx_for_raw_decode.config.write();
        if cfg.data.raw_full_decode != active {
            cfg.data.raw_full_decode = active;
            cfg.save();
        }
    });

    let ctx_for_raw_cache = ctx.clone();
    raw_cache_row.connect_active_notify(move |row| {
        let active = row.is_active();
        crate::library::set_raw_cache_enabled(active);
        let mut cfg = ctx_for_raw_cache.config.write();
        if cfg.data.raw_decode_cache_enabled != active {
            cfg.data.raw_decode_cache_enabled = active;
            cfg.save();
        }
    });

    let pending_disk_cache_save: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let ctx_for_disk_cache = ctx.clone();
    disk_cache_row.connect_value_notify(move |row| {
        let new_value = row.value() as u32;
        if let Some(id) = pending_disk_cache_save.take() {
            id.remove();
        }
        let ctx_for_save = ctx_for_disk_cache.clone();
        let pending = pending_disk_cache_save.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            pending.set(None);
            let mut cfg = ctx_for_save.config.write();
            if cfg.data.cache_disk_cap_mb != new_value {
                cfg.data.cache_disk_cap_mb = new_value;
                cfg.save();
            }
        });
        pending_disk_cache_save.set(Some(id));
    });

    // --- WATCH FOLDERS GROUP ---
    let folders_group = adw::PreferencesGroup::builder()
        .title("Watch Folders")
        .description("Pick folders to sync.")
        .build();
    settings_page.add(&folders_group);

    let startup_state = Rc::new(RefCell::new(config.data.run_on_startup));
    let background_sync_state = Rc::new(RefCell::new(config.data.background_sync_enabled));
    let apply_in_flight = Rc::new(Cell::new(false));
    let tracked_rows = Rc::new(RefCell::new(Vec::<FolderRowData>::new()));
    let albums: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));

    let apply_settings = Rc::new(clone!(
        #[weak]
        window,
        #[weak]
        startup_row,
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
        metered_row,
        #[weak]
        battery_row,
        #[weak]
        notifications_row,
        #[weak]
        library_view_row,
        #[weak]
        preview_full_row,
        #[weak]
        raw_full_decode_row,
        #[weak]
        raw_cache_row,
        #[weak]
        disk_cache_row,
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
        background_sync_row,
        #[weak]
        xmp_sidecar_row,
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
        #[strong]
        live_watch_paths,
        #[strong]
        sync_now_tx,
        #[strong]
        startup_state,
        #[strong]
        background_sync_state,
        #[strong]
        apply_in_flight,
        #[strong]
        shared_config,
        move |include_connectivity: bool, show_success_ack: bool| {
            if apply_in_flight.get() {
                return;
            }
            apply_in_flight.set(true);

            let (
                mut internal_url_enabled,
                mut external_url_enabled,
                mut internal_url,
                mut external_url,
                mut api_key,
            ) = {
                let existing = shared_config.read();
                (
                    existing.data.internal_url_enabled,
                    existing.data.external_url_enabled,
                    existing.data.internal_url.clone(),
                    existing.data.external_url.clone(),
                    existing.get_api_key().unwrap_or_default(),
                )
            };
            if include_connectivity {
                internal_url_enabled = internal_switch.is_active();
                external_url_enabled = external_switch.is_active();
                internal_url = internal_entry.text().to_string();
                external_url = external_entry.text().to_string();
                api_key = api_key_entry.text().to_string();

                // Reject non-HTTP(S) URL schemes before persisting.
                if internal_url_enabled
                    && !internal_url.trim().is_empty()
                    && let Err(err) = crate::sanitize::validate_http_url(&internal_url)
                {
                    show_alert(&window, "Invalid Internal URL", &err);
                    apply_in_flight.set(false);
                    return;
                }
                if external_url_enabled
                    && !external_url.trim().is_empty()
                    && let Err(err) = crate::sanitize::validate_http_url(&external_url)
                {
                    show_alert(&window, "Invalid External URL", &err);
                    apply_in_flight.set(false);
                    return;
                }
            }
            let run_on_startup = startup_row.is_active();
            let pause_on_metered_network = metered_row.is_active();
            let pause_on_battery_power = battery_row.is_active();
            let notifications_enabled = notifications_row.is_active();
            let library_view_enabled = library_view_row.is_active();
            let library_preview_full_resolution = preview_full_row.is_active();
            let raw_decode_cache_enabled = raw_cache_row.is_active();
            let raw_full_decode = raw_full_decode_row.is_active();
            let cache_disk_cap_mb = disk_cache_row.value() as u32;
            let upload_concurrency = concurrency_row.value() as u8;
            let quiet_hours_enabled = quiet_hours_row.is_active();
            let quiet_hours_start = quiet_hours_enabled.then(|| quiet_start_row.value() as u8);
            let quiet_hours_end = quiet_hours_enabled.then(|| quiet_end_row.value() as u8);
            let background_sync_enabled = background_sync_row.is_active();
            let upload_xmp_sidecars = xmp_sidecar_row.is_active();
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

                let is_default = album_name.is_empty() || album_name == DEFAULT_ALBUM_LABEL;
                let resolved_album_name = if is_default {
                    Path::new(&folder)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                } else {
                    Some(album_name)
                };

                if is_default && !has_rules && resolved_album_name.is_none() {
                    watch_paths.push(WatchPathEntry::Simple(folder));
                } else {
                    let album_id = resolved_album_name
                        .as_ref()
                        .and_then(|n| albums_map.get(n).cloned());
                    watch_paths.push(WatchPathEntry::WithConfig {
                        path: folder,
                        album_id,
                        album_name: resolved_album_name,
                        rules,
                    });
                }
            }

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

            let previous_startup = *startup_state.borrow();
            let previous_background_sync = *background_sync_state.borrow();

            glib::MainContext::default().spawn_local(clone!(
                #[weak]
                window,
                #[weak]
                startup_row,
                #[strong]
                shared_state,
                #[strong]
                api_client,
                #[strong]
                queue_manager,
                #[strong]
                monitor_handle,
                #[strong]
                albums,
                #[strong]
                live_watch_paths,
                #[strong]
                sync_now_tx,
                #[strong]
                startup_state,
                #[strong]
                background_sync_state,
                #[strong]
                apply_in_flight,
                #[strong]
                shared_config,
                async move {
                    if run_on_startup != previous_startup {
                        match autostart::apply(&window, run_on_startup).await {
                            Ok(granted) if granted == run_on_startup => {}
                            Ok(_) => {
                                startup_row.set_active(previous_startup);
                                apply_in_flight.set(false);

                                show_alert(
                                    &window,
                                    "Startup Permission Needed",
                                    "Mimick was not allowed to start automatically at login.",
                                );
                                return;
                            }
                            Err(err) => {
                                startup_row.set_active(previous_startup);
                                apply_in_flight.set(false);

                                show_alert(&window, "Could Not Update Startup Setting", &err);
                                return;
                            }
                        }
                    }

                    {
                        let mut new_config = shared_config.write();
                        new_config.data.internal_url_enabled = internal_url_enabled;
                        new_config.data.external_url_enabled = external_url_enabled;
                        new_config.data.internal_url = internal_url;
                        new_config.data.external_url = external_url;
                        new_config.data.watch_paths = watch_paths.clone();
                        new_config.data.run_on_startup = run_on_startup;
                        new_config.data.background_sync_enabled = background_sync_enabled;
                        new_config.data.pause_on_metered_network = pause_on_metered_network;
                        new_config.data.pause_on_battery_power = pause_on_battery_power;
                        new_config.data.notifications_enabled = notifications_enabled;
                        new_config.data.library_view_enabled = library_view_enabled;
                        new_config.data.library_preview_full_resolution =
                            library_preview_full_resolution;
                        new_config.data.raw_decode_cache_enabled = raw_decode_cache_enabled;
                        crate::library::set_raw_cache_enabled(raw_decode_cache_enabled);
                        new_config.data.raw_full_decode = raw_full_decode;
                        crate::library::set_raw_full_decode(raw_full_decode);
                        new_config.data.cache_disk_cap_mb = cache_disk_cap_mb;
                        new_config.data.startup_catchup_mode = catchup_mode;
                        new_config.data.upload_concurrency = upload_concurrency;
                        new_config.data.quiet_hours_start = quiet_hours_start;
                        new_config.data.quiet_hours_end = quiet_hours_end;
                        new_config.data.upload_xmp_sidecars = upload_xmp_sidecars;

                        if include_connectivity
                            && !api_key.is_empty()
                            && !new_config.set_api_key(&api_key)
                        {
                            apply_in_flight.set(false);

                            show_alert(
                                &window,
                                "Could Not Save API Key",
                                "Mimick could not store the API key in your desktop keyring.",
                            );
                            return;
                        }

                        if !new_config.save() {
                            apply_in_flight.set(false);

                            show_alert(
                                &window,
                                "Could Not Save Settings",
                                "Mimick could not write the updated configuration to disk.",
                            );
                            return;
                        }
                    }

                    *startup_state.borrow_mut() = run_on_startup;
                    *background_sync_state.borrow_mut() = background_sync_enabled;

                    api_client
                        .update_settings(
                            runtime_internal_url.clone(),
                            runtime_external_url.clone(),
                            api_key.clone(),
                        )
                        .await;

                    if include_connectivity && !api_key.is_empty() {
                        match api_client.get_all_albums().await {
                            Ok(fetched) => {
                                *albums.borrow_mut() = fetched;
                            }
                            Err(err) => {
                                log::warn!("Could not fetch albums after saving settings: {}", err);
                            }
                        }
                    }

                    queue_manager.set_worker_limit(upload_concurrency);
                    queue_manager.update_environment_policy(
                        crate::queue_manager::EnvironmentPolicy {
                            pause_on_metered_network,
                            pause_on_battery_power,
                            quiet_hours_start,
                            quiet_hours_end,
                        },
                    );

                    crate::notifications::set_enabled(notifications_enabled);

                    if previous_background_sync != background_sync_enabled {
                        let mut state = shared_state.lock();
                        if !background_sync_enabled && state.status != "uploading" && !state.paused
                        {
                            state.status = "idle".to_string();
                            state.pause_reason = None;
                        }
                    }

                    let monitor_paths = if background_sync_enabled {
                        watch_paths.clone()
                    } else {
                        Vec::new()
                    };
                    monitor_handle.replace_watch_paths(monitor_paths, background_sync_enabled);

                    *live_watch_paths.lock() = watch_paths.clone();

                    if background_sync_enabled
                        && previous_background_sync != background_sync_enabled
                    {
                        let _ = sync_now_tx.send(());
                    }

                    {
                        let mut state = shared_state.lock();
                        state.watched_folder_count = watch_paths.len();
                        let current_paths = watch_paths
                            .iter()
                            .map(|entry| entry.path().to_string())
                            .collect::<std::collections::HashSet<_>>();
                        state
                            .folder_statuses
                            .retain(|path, _| current_paths.contains(path));
                    }

                    apply_in_flight.set(false);
                    if show_success_ack {
                        show_alert(
                            &window,
                            "Settings Saved",
                            "Mimick saved the updated settings successfully.",
                        );
                    }
                }
            ));
        }
    ));

    let auto_apply_settings: Rc<dyn Fn()> = Rc::new(clone!(
        #[strong]
        apply_settings,
        move || {
            (apply_settings)(false, false);
        }
    ));

    // Reuse the application-wide API client — do NOT create a new one here.
    // Creating a new reqwest Client per window open allocates a new connection pool
    // that takes ~30s to self-clean, causing RAM to grow with each open/close cycle.
    let albums_ref = albums.clone();

    // Downgrade the window to a weak ref BEFORE the spawn.
    // After the async await, we upgrade it — if it's None the window was closed
    // while the API call was in-flight. We bail immediately, releasing all strong
    // refs to FolderRowData (and their contained GTK widgets) so they can be freed.
    // Without this, rapid open/close cycles would accumulate orphaned widget sets.
    let weak_win = window.downgrade();
    let client = api_client.clone();

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

    // List FIRST (matching Python layout), then Add button below
    let folders_list = ListBox::builder()
        .margin_top(12)
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    folders_group.add(&folders_list);

    let add_folder_btn = Button::builder().label("Add Folder").margin_top(12).build();
    folders_group.add(&add_folder_btn);

    let folder_default_catchup = config.data.startup_catchup_mode.clone();

    // Add existing paths to listbox with album dropdown
    for entry in &config.data.watch_paths {
        add_folder_row(
            &folders_list,
            entry,
            folder_default_catchup.clone(),
            albums.clone(),
            &tracked_rows,
            auto_apply_settings.clone(),
        );
    }

    let folders_list_clone = folders_list.clone();
    let window_clone = window.clone();
    let tracked_rows_clone = tracked_rows.clone();
    let albums_clone = albums.clone();
    let apply_settings_for_add = auto_apply_settings.clone();
    let folder_default_catchup_for_add = folder_default_catchup.clone();

    add_folder_btn.connect_clicked(move |_| {
        let dialog = FileDialog::builder().title("Select Watch Folder").build();
        let list_clone = folders_list_clone.clone();
        let tracked_clone = tracked_rows_clone.clone();
        let albums_ref = albums_clone.clone();
        let apply_settings_for_add = apply_settings_for_add.clone();
        let folder_default_catchup_for_add = folder_default_catchup_for_add.clone();

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
                    add_folder_row(
                        &list_clone,
                        &WatchPathEntry::Simple(path_str),
                        folder_default_catchup_for_add.clone(),
                        albums_ref.clone(),
                        &tracked_clone,
                        apply_settings_for_add.clone(),
                    );
                    (apply_settings_for_add)();
                }
            },
        );
    });

    let actions_ui::ActionsWidgets {
        sync_now_btn,
        pause_btn,
        queue_btn,
        export_btn,
        clear_cache_btn,
        quit_btn,
    } = actions_ui::build_actions_group(&status_page, &settings_page);

    pause_btn.set_label(if queue_manager.is_paused() {
        "Resume"
    } else {
        "Pause"
    });

    let qm_for_inspector = queue_manager.clone();
    queue_btn.connect_clicked(clone!(
        #[weak]
        window,
        move |_| {
            show_queue_inspector(&window, qm_for_inspector.clone());
        }
    ));

    let qm_for_pause = queue_manager.clone();
    pause_btn.connect_clicked(clone!(
        #[weak]
        pause_btn,
        move |_| {
            let paused = !qm_for_pause.is_paused();
            qm_for_pause.set_paused(paused, paused.then(|| "Paused by user".to_string()));
            pause_btn.set_label(if paused { "Resume" } else { "Pause" });
        }
    ));

    sync_now_btn.connect_clicked(move |_| {
        let _ = sync_now_tx.send(());
    });

    export_btn.connect_clicked(clone!(
        #[weak]
        window,
        #[strong]
        shared_state,
        #[strong]
        shared_config,
        move |_| {
            let dialog = FileDialog::builder()
                .title("Choose Diagnostics Export Folder")
                .build();
            let state = shared_state.clone();
            let config_ref = shared_config.clone();
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
                            let state_snapshot = state.lock().clone();
                            let config_snapshot = config_ref.read().clone();
                            glib::MainContext::default().spawn_local(clone!(
                                #[weak]
                                window,
                                async move {
                                    let export_result = tokio::task::spawn_blocking(move || {
                                        diagnostics::export_bundle(
                                            &path,
                                            &state_snapshot,
                                            &config_snapshot,
                                        )
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

                                    show_alert(&window, heading, &body);
                                }
                            ));
                        }
                    }
                ),
            );
        }
    ));

    clear_cache_btn.connect_clicked(clone!(
        #[weak]
        window,
        move |_| {
            // Drop the in-memory thumbnail textures immediately, then wipe
            // every on-disk cache subdirectory off the UI thread.
            let _ = thumbnail_cache.clear();
            let window_for_done = window.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = tokio::task::spawn_blocking(crate::cache_manager::clear_all_blocking)
                    .await
                    .map_err(|err| err.to_string())
                    .and_then(|inner| inner);
                let (heading, body) = match result {
                    Ok(()) => (
                        "Cache Cleared",
                        "Removed thumbnails, decoded RAW previews, EXIF, video, and preview caches."
                            .to_string(),
                    ),
                    Err(err) => ("Could Not Clear Cache", err),
                };
                show_alert(&window_for_done, heading, &body);
            });
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
        #[strong]
        apply_settings,
        move |_| {
            (apply_settings)(true, true);
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
    background_sync_row.set_active(config.data.background_sync_enabled);
    notifications_row.set_active(config.data.notifications_enabled);
    library_view_row.set_active(config.data.library_view_enabled);
    preview_full_row.set_active(config.data.library_preview_full_resolution);
    raw_cache_row.set_active(config.data.raw_decode_cache_enabled);
    raw_full_decode_row.set_active(config.data.raw_full_decode);
    raw_cache_row.set_sensitive(config.data.raw_full_decode);
    if config.data.cache_disk_cap_mb > 0 {
        disk_cache_row.set_value(config.data.cache_disk_cap_mb as f64);
    }
    concurrency_row.set_value(config.data.upload_concurrency as f64);
    xmp_sidecar_row.set_active(config.data.upload_xmp_sidecars);
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
                show_alert(
                    &window,
                    "Invalid Selection",
                    "At least one URL (Internal or External) must be enabled.",
                );
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
                show_alert(
                    &window,
                    "Invalid Selection",
                    "At least one URL (Internal or External) must be enabled.",
                );
            }
            external_entry.set_sensitive(switch.is_active());
        }
    ));

    startup_row.connect_active_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    metered_row.connect_active_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    battery_row.connect_active_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    notifications_row.connect_active_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    catchup_row.connect_selected_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    concurrency_row.connect_value_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    quiet_hours_row.connect_active_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    quiet_start_row.connect_value_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    quiet_end_row.connect_value_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    background_sync_row.connect_active_notify(clone!(
        #[strong]
        auto_apply_settings,
        move |_| {
            (auto_apply_settings)();
        }
    ));

    // If background sync is disabled AND this is the only open window, closing
    // settings should exit the app. When the library window is also open we
    // must not quit — the user explicitly opened settings *from* the library
    // and expects the library to stay around after dismissing settings.
    window.connect_close_request(clone!(
        #[strong]
        app_clone,
        #[strong]
        ctx,
        move |_| {
            // Read current background-sync state directly from config rather
            // than from a shadow RefCell, so any code path that mutates the
            // config (apply_settings, future autosave, etc.) is reflected
            // here without a separate book-keeping step.
            // The closing window is still in app.windows() at this point, so
            // a count of 1 means we're the only window left — quit the app.
            // > 1 means another window (typically the library) is open and
            // should keep running.
            let bg_sync = ctx.config.read().data.background_sync_enabled;
            if !bg_sync && app_clone.windows().len() <= 1 {
                app_clone.quit();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    ));

    // Background state poller — reads directly from in-memory shared state.
    // No disk I/O; the timer tears itself down when the window is destroyed.
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
                    folder_subtitles,
                ) = {
                    let s = shared_state.lock();
                    let folder_subtitles = tracked_rows
                        .borrow()
                        .iter()
                        .map(|row_data| {
                            let mut final_subtitle = row_data.base_subtitle.clone();
                            if !final_subtitle.is_empty() {
                                final_subtitle.push('\n');
                            }

                            if let Some(folder_status) = s.folder_statuses.get(&row_data.path) {
                                if let Some(err) = &folder_status.last_error {
                                    final_subtitle.push_str(&format!("Error: {}", err));
                                } else {
                                    let mut txt =
                                        format!("Pending: {}", folder_status.pending_count);
                                    if let Some(t) = folder_status.last_sync_at {
                                        txt.push_str(&format!(
                                            " - Last Sync: {}",
                                            format_sync_age(Some(t))
                                        ));
                                    }
                                    final_subtitle.push_str(&txt);
                                }
                            } else {
                                final_subtitle.push_str("Status: Idle");
                            }

                            (row_data.action_row.clone(), final_subtitle)
                        })
                        .collect::<Vec<_>>();

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
                        folder_subtitles,
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
                for (row, subtitle) in folder_subtitles {
                    row.set_subtitle(&subtitle);
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
    window.present();
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
