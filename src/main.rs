//! Handles application bootstrap, single-instance wiring, and daemon startup flow.

use gtk::prelude::*;
use libadwaita as adw;
use log::Record;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

mod api_client;
mod app_context;
mod autostart;
mod config;
mod diagnostics;
mod library;
mod monitor;
mod notifications;
mod queue_manager;
mod runtime_env;
mod settings_window;
mod startup_scan;
mod state_manager;
mod sync_index;
mod tray_icon;
mod watch_path_display;

use api_client::ImmichApiClient;
use app_context::AppContext;
use config::{Config, best_matching_watch_entry};
use library::state::LibraryState;
use library::thumbnail_cache::ThumbnailCache;
use monitor::Monitor;
use queue_manager::{EnvironmentPolicy, FileTask, QueueManager};
use settings_window::build_settings_window;
use startup_scan::queue_unsynced_files;
use state_manager::{AppState, StateManager};
use sync_index::SyncIndex;
use tray_icon::build_tray;

use flexi_logger::{
    Cleanup, Criterion, DeferredNow, Duplicate, FileSpec, Logger, Naming, WriteMode, style,
};
use std::io::Write;

/// Shared application context reused by UI entry points and the shutdown path.
static APP_CONTEXT: std::sync::OnceLock<Arc<AppContext>> = std::sync::OnceLock::new();

fn format_log_location(record: &Record) -> String {
    match (record.file(), record.line()) {
        (Some(file), Some(line)) => format!(" {}:{}", file, line),
        _ => String::new(),
    }
}

fn detailed_plain_format(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {:<5} [{}] {}{}",
        now.format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        record.level(),
        record.target(),
        record.args(),
        format_log_location(record)
    )
}

fn detailed_colored_format(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {} [{}] {}{}",
        now.format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        style(record.level()).paint(format!("{:<5}", record.level())),
        record.target(),
        record.args(),
        format_log_location(record)
    )
}

#[tokio::main]
async fn main() {
    // Mirror logs to stdout and to a rotating cache file for easier support/debugging.
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("mimick");

    let _logger = Logger::try_with_env_or_str("info")
        .expect("Failed to parse log level")
        .log_to_file(
            FileSpec::default()
                .directory(log_dir)
                .basename("mimick")
                .suppress_timestamp() // "mimick.log" instead of "mimick_2026-03-09_10-33-35.log"
                .suffix("log"),
        )
        .format_for_files(detailed_plain_format)
        .format_for_stdout(detailed_colored_format)
        .rotate(
            Criterion::Size(2_000_000),
            Naming::Numbers,
            Cleanup::KeepLogFiles(5),
        )
        // Also print to stdout for systemd / terminal users
        .duplicate_to_stdout(Duplicate::All)
        .write_mode(WriteMode::Direct)
        .start()
        .expect("Failed to initialize logger");

    let app = adw::Application::builder()
        .application_id("dev.nicx.mimick")
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let is_primary_instance = Arc::new(AtomicBool::new(false));
    let is_primary_instance_clone = is_primary_instance.clone();

    let shared_state: Arc<Mutex<AppState>> = Arc::new(Mutex::new({
        let mut saved = StateManager::new().read_state();
        // Any items left in the channel during shutdown were dropped, so we must
        // sync total_queued down to processed_count to clear the stuck queue state.
        saved.total_queued = saved.processed_count;
        saved.queue_size = 0;
        saved.failed_count = 0; // Will be repopulated from retries.json if any
        saved.current_file = None;

        for status in saved.folder_statuses.values_mut() {
            status.pending_count = 0;
        }

        // Reset volatile fields that shouldn't survive a restart
        AppState {
            status: "idle".to_string(),
            active_workers: 0,
            ..saved
        }
    }));

    let shared_state_startup = shared_state.clone();

    // Only the primary instance should initialize background services.
    // Secondary launches remote-control the primary through GTK's single-instance support.
    app.connect_startup(move |app| {
        is_primary_instance_clone.store(true, Ordering::SeqCst);

        log::info!("Mimick primary instance initializing");
        // Always follow the desktop's light/dark preference.
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);

        thread_local! {
            static APP_HOLD: std::cell::RefCell<Option<gtk::gio::ApplicationHoldGuard>> = const { std::cell::RefCell::new(None) };
        }
        APP_HOLD.with(|hold| {
            *hold.borrow_mut() = Some(app.hold());
        });

        // Load config
        let config = Config::new();
        let watch_folder_count = config.data.watch_paths.len();
        let live_watch_paths = Arc::new(Mutex::new(config.data.watch_paths.clone()));
        log::info!(
            "Config: internal={} external={} paths={:?}",
            config.data.internal_url,
            config.data.external_url,
            config.watch_path_strings(),
        );

        {
            let mut state = shared_state_startup.lock().unwrap();
            state.watched_folder_count = watch_folder_count;
        }

        let background_sync_enabled = config.data.background_sync_enabled;

        let api_key = config.get_api_key().unwrap_or_default();
        let runtime_internal_url = if config.data.internal_url_enabled {
            config.data.internal_url.clone()
        } else {
            String::new()
        };
        let runtime_external_url = if config.data.external_url_enabled {
            config.data.external_url.clone()
        } else {
            String::new()
        };

        let api_client = Arc::new(ImmichApiClient::new(
            runtime_internal_url,
            runtime_external_url,
            api_key,
        ));
        let sync_index = Arc::new(Mutex::new(SyncIndex::new()));

        let sync_index_flusher = sync_index.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                if let Ok(mut index) = sync_index_flusher.lock() {
                    let _ = index.flush();
                }
            }
        });

        let qm = Arc::new(QueueManager::new(
            api_client.clone(),
            config.data.upload_concurrency.max(1) as usize,
            shared_state_startup.clone(),
            sync_index.clone(),
            EnvironmentPolicy {
                pause_on_metered_network: config.data.pause_on_metered_network,
                pause_on_battery_power: config.data.pause_on_battery_power,
                quiet_hours_start: config.data.quiet_hours_start,
                quiet_hours_end: config.data.quiet_hours_end,
            },
        ));

        // Apply the user's notification preference before any notification can fire.
        crate::notifications::set_enabled(config.data.notifications_enabled);

        // Keep the watcher service alive, but optionally disable active folder watches.
        let (tx, mut rx) = mpsc::channel(32);
        let monitor_paths = if background_sync_enabled {
            config.data.watch_paths.clone()
        } else {
            Vec::new()
        };
        let monitor = Monitor::new(monitor_paths, background_sync_enabled);
        let monitor_handle = Arc::new(monitor.start(tx));
        if background_sync_enabled {
            log::info!("File monitor started");
        } else {
            log::info!("Background sync is disabled; monitor started with no active watches");
        }

        // Feed monitor events into the upload queue, preserving per-path album config
        let qm_clone = qm.clone();
        let live_watch_paths_for_queue = live_watch_paths.clone();
        tokio::spawn(async move {
            while let Some((path, checksum)) = rx.recv().await {
                log::info!("Queuing: {} (sha1={})", path, checksum);

                let (album_id, album_name, watch_path) = {
                    let path_configs = live_watch_paths_for_queue.lock().unwrap();
                    best_matching_watch_entry(std::path::Path::new(&path), &path_configs)
                        .map(|entry| match entry {
                            config::WatchPathEntry::WithConfig {
                                album_id,
                                album_name,
                                ..
                            } => (
                                album_id.clone(),
                                album_name.clone(),
                                entry.path().to_string(),
                            ),
                            config::WatchPathEntry::Simple(_) => {
                                (None, None, entry.path().to_string())
                            }
                        })
                        .unwrap_or((None, None, String::new()))
                };

                let _ = qm_clone
                    .add_to_queue(FileTask {
                        path,
                        watch_path,
                        checksum,
                        album_id,
                        album_name,
                        reassociate_only: false,
                    })
                    .await;
            }
        });

        let startup_qm = qm.clone();
        let startup_paths = config.data.watch_paths.clone();
        let startup_sync_index = sync_index.clone();
        let (manual_sync_tx, mut manual_sync_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let thumbnail_cache = Arc::new(ThumbnailCache::with_capacity_mb(
            api_client.clone(),
            config.data.library_thumbnail_cache_mb,
        ));
        let library_state = Arc::new(Mutex::new(LibraryState::new()));
        let ctx = Arc::new(AppContext {
            state: shared_state_startup.clone(),
            api_client: api_client.clone(),
            queue_manager: qm.clone(),
            monitor_handle: monitor_handle.clone(),
            sync_index: sync_index.clone(),
            live_watch_paths: live_watch_paths.clone(),
            sync_now_tx: manual_sync_tx.clone(),
            thumbnail_cache,
            library_state,
            library_timeline_active: std::sync::atomic::AtomicBool::new(false),
        });
        let _ = APP_CONTEXT.set(ctx.clone());

        // The startup scan backfills anything that arrived while Mimick was not running.
        if background_sync_enabled {
            let shared_state_startup_task = shared_state_startup.clone();
            let startup_api = ctx.api_client.clone();
            tokio::spawn(async move {
                queue_unsynced_files(
                    startup_paths,
                    startup_qm,
                    startup_sync_index,
                    startup_api,
                    config::Config::new().data.startup_catchup_mode,
                    shared_state_startup_task,
                )
                .await;
            });
        } else {
            log::info!("Background sync is disabled; skipping startup catch-up scan");
        }

        let startup_state = shared_state_startup.clone();
        let status_api = ctx.api_client.clone();
        tokio::spawn(async move {
            let connected = status_api.check_connection().await;
            let route = status_api.active_route_label().await;
            let latest_issue = status_api.latest_issue().await;

            let mut state = startup_state.lock().unwrap();
            state.active_server_route = route;
            if connected {
                state.last_error = None;
                state.last_error_guidance = None;
            } else if let Some(issue) = latest_issue {
                state.last_error = Some(issue.summary);
                state.last_error_guidance = Some(issue.guidance);
            }
        });

        let manual_qm = qm.clone();
        let manual_sync_index = sync_index.clone();
        let manual_sync_api = ctx.api_client.clone();
        let shared_state_manual_task = shared_state_startup.clone();
        tokio::spawn(async move {
            while manual_sync_rx.recv().await.is_some() {
                let config = Config::new();
                queue_unsynced_files(
                    config.data.watch_paths.clone(),
                    manual_qm.clone(),
                    manual_sync_index.clone(),
                    manual_sync_api.clone(),
                    config.data.startup_catchup_mode,
                    shared_state_manual_task.clone(),
                )
                .await;
            }
        });

        let app_clone2 = app.clone();
        let app_clone3 = app.clone();

        // Cross-thread flag: Tokio sets it; the GTK timer reads and clears it.
        // Arc<Mutex<bool>> is Send + Sync, so it can cross the tokio::spawn boundary.
        let settings_flag = Arc::new(std::sync::Mutex::new(false));
        let settings_flag_writer = settings_flag.clone(); // moves into tokio::spawn (Send ✓)
        let quit_flag = Arc::new(std::sync::Mutex::new(false));
        let quit_flag_writer = quit_flag.clone(); // moves into tokio::spawn (Send ✓)
        let pause_flag = Arc::new(std::sync::Mutex::new(false));
        let pause_flag_writer = pause_flag.clone();
        let sync_now_flag = Arc::new(std::sync::Mutex::new(false));
        let sync_now_flag_writer = sync_now_flag.clone();

        // GTK-side: poll the flag every 250ms on the main thread.
        // The application handle stays on the GTK thread and never enters Tokio tasks.
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let settings_triggered = {
                let mut f = settings_flag.lock().unwrap();
                if *f {
                    *f = false;
                    true
                } else {
                    false
                }
            };
            if settings_triggered {
                let ctx = APP_CONTEXT
                    .get()
                    .cloned()
                    .expect("App context should be initialized before opening settings");
                open_settings_if_needed(&app_clone2, ctx);
            }

            let quit_triggered = {
                let mut f = quit_flag.lock().unwrap();
                if *f {
                    *f = false;
                    true
                } else {
                    false
                }
            };
            if quit_triggered {
                app_clone3.quit();
                return glib::ControlFlow::Break;
            }

            let pause_triggered = {
                let mut f = pause_flag.lock().unwrap();
                if *f {
                    *f = false;
                    true
                } else {
                    false
                }
            };
            if pause_triggered {
                let qm = &APP_CONTEXT
                    .get()
                    .expect("App context should be initialized before pause handling")
                    .queue_manager;
                let paused = !qm.is_paused();
                let reason = if paused {
                    Some("Paused by user".to_string())
                } else {
                    None
                };
                qm.set_paused(paused, reason);
            }

            let sync_now_triggered = {
                let mut f = sync_now_flag.lock().unwrap();
                if *f {
                    *f = false;
                    true
                } else {
                    false
                }
            };
            if sync_now_triggered {
                let tx = &APP_CONTEXT
                    .get()
                    .expect("App context should be initialized before manual sync handling")
                    .sync_now_tx;
                let _ = tx.send(());
            }

            glib::ControlFlow::Continue
        });

        // Tokio-side: build the tray and forward watch signals into the flag.
        // Only *_writer flags (Send ✓) and watch receivers (Send ✓) are captured here.
        tokio::spawn(async move {
            log::info!("Starting system tray");
            match build_tray().await {
                Ok((_handle, mut settings_rx, mut quit_rx, mut pause_rx, mut sync_now_rx)) => {
                    loop {
                        tokio::select! {
                            res = settings_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *settings_rx.borrow() {
                                    *settings_flag_writer.lock().unwrap() = true;
                                }
                            }
                            res = quit_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *quit_rx.borrow() {
                                    *quit_flag_writer.lock().unwrap() = true;
                                }
                            }
                            res = pause_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *pause_rx.borrow() {
                                    *pause_flag_writer.lock().unwrap() = true;
                                }
                            }
                            res = sync_now_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *sync_now_rx.borrow() {
                                    *sync_now_flag_writer.lock().unwrap() = true;
                                }
                            }
                        }
                    }
                }
                Err(e) => log::warn!("System tray failed to start: {:?}", e),
            }
        });
    });

    // Handle command line from both the primary and secondary instances.
    app.connect_command_line(move |app, cmdline| {
        let argv: Vec<String> = cmdline
            .arguments()
            .iter()
            .filter_map(|a| a.to_str().map(|s| s.to_string()))
            .collect();

        let quit_requested = argv.contains(&"--quit".to_string());
        if quit_requested {
            app.quit();
            return 0.into();
        }

        let runtime_config = Config::new();
        let open_settings = argv.contains(&"--settings".to_string())
            // Also open settings when activated by a secondary instance (e.g. clicking
            // the app icon in the launcher while the daemon is already running).
            || cmdline.is_remote()
            || runtime_config.get_api_key().unwrap_or_default().is_empty()
            || !runtime_config.data.background_sync_enabled;

        if open_settings {
            let ctx = APP_CONTEXT
                .get()
                .cloned()
                .expect("App context should be initialized before command-line activation");
            open_settings_if_needed(app, ctx);
        }

        app.activate();
        0.into()
    });

    app.connect_activate(move |_app| {
        log::debug!("App activated");
    });

    log::info!("GTK application starting up");
    app.run();

    // Persist final state and any pending retries on graceful shutdown.
    if is_primary_instance.load(Ordering::SeqCst) {
        if let Some(ctx) = APP_CONTEXT.get() {
            ctx.queue_manager.flush_retries();
        }
        let state = APP_CONTEXT
            .get()
            .map(|ctx| ctx.state.lock().unwrap().clone())
            .unwrap_or_else(|| shared_state.lock().unwrap().clone());
        StateManager::new().write_state(state);
        log::info!("Mimick exiting");
    }
}

/// Open the settings window only if one is not already visible.
fn open_settings_if_needed(app: &adw::Application, ctx: Arc<AppContext>) {
    if let Some(win) = app.windows().first() {
        win.present();
    } else if Config::new().data.library_view_enabled {
        log::debug!("Opening library window");
        library::build_library_window(app, ctx);
    } else {
        log::debug!("Opening settings window");
        build_settings_window(app, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FolderRules, WatchPathEntry};

    #[test]
    fn test_live_queue_matching_prefers_most_specific_watch_path() {
        let entries = vec![
            WatchPathEntry::WithConfig {
                path: "/home/user/Pictures".into(),
                album_id: Some("root-album".into()),
                album_name: Some("Pictures".into()),
                rules: FolderRules::default(),
            },
            WatchPathEntry::WithConfig {
                path: "/home/user/Pictures/Trips".into(),
                album_id: Some("trips-album".into()),
                album_name: Some("Trips".into()),
                rules: FolderRules::default(),
            },
        ];

        let matched = best_matching_watch_entry(
            std::path::Path::new("/home/user/Pictures/Trips/day1/photo.jpg"),
            &entries,
        )
        .unwrap();

        let config::WatchPathEntry::WithConfig { album_id, .. } = matched else {
            panic!("expected configured watch entry");
        };
        assert_eq!(album_id.as_deref(), Some("trips-album"));
        assert_eq!(matched.album_name(), Some("Trips"));
    }
}
