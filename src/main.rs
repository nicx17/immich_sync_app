//! Application bootstrap, single-instance wiring, and daemon startup flow.

use gtk::prelude::*;
use libadwaita as adw;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

mod api_client;
mod autostart;
mod config;
mod monitor;
mod notifications;
mod queue_manager;
mod restart;
mod settings_window;
mod startup_scan;
mod state_manager;
mod sync_index;
mod tray_icon;
mod watch_path_display;

use api_client::ImmichApiClient;
use config::Config;
use monitor::Monitor;
use queue_manager::{FileTask, QueueManager};
use restart::{launch_replacement, take_restart_request};
use settings_window::build_settings_window;
use startup_scan::queue_unsynced_files;
use state_manager::{AppState, StateManager};
use sync_index::SyncIndex;
use tray_icon::build_tray;

use flexi_logger::{FileSpec, Logger, WriteMode, colored_detailed_format, detailed_format};

/// Queue manager handle retained so the graceful shutdown path can flush pending retries.
static QM_HANDLE: std::sync::OnceLock<Arc<QueueManager>> = std::sync::OnceLock::new();
/// Shared API client reused by the settings window and startup scan.
static API_CLIENT_HANDLE: std::sync::OnceLock<Arc<ImmichApiClient>> = std::sync::OnceLock::new();

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
        .format_for_files(detailed_format)
        .format_for_stdout(colored_detailed_format)
        // Also print to stdout for systemd / terminal users
        .duplicate_to_stdout(flexi_logger::Duplicate::All)
        .write_mode(WriteMode::Direct)
        .start()
        .expect("Failed to initialize logger");

    let app = adw::Application::builder()
        .application_id("io.github.nicx17.mimick")
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let is_primary_instance = Arc::new(AtomicBool::new(false));
    let is_primary_instance_clone = is_primary_instance.clone();

    // Workers update state in memory and the UI reads the same shared snapshot.
    // Disk-backed status is only used for startup recovery and graceful shutdown.
    let shared_state: Arc<Mutex<AppState>> = Arc::new(Mutex::new({
        let saved = StateManager::new().read_state();
        // Reset volatile fields that shouldn't survive a restart
        AppState {
            status: "idle".to_string(),
            active_workers: 0,
            ..saved
        }
    }));

    let shared_state_startup = shared_state.clone();
    let shared_state_cmdline = shared_state.clone();

    // Only the primary instance should initialize background services.
    // Secondary launches remote-control the primary through GTK's single-instance support.
    app.connect_startup(move |app| {
        is_primary_instance_clone.store(true, Ordering::SeqCst);

        log::info!("Mimick primary instance initializing");

        // Keep the process alive when the settings window is hidden.
        Box::leak(Box::new(app.hold()));

        // Load config
        let config = Config::new();
        log::info!(
            "Config: internal={} external={} paths={:?}",
            config.data.internal_url,
            config.data.external_url,
            config.watch_path_strings(),
        );

        let api_key = config.get_api_key().unwrap_or_default();

        let api_client = Arc::new(ImmichApiClient::new(
            config.data.internal_url.clone(),
            config.data.external_url.clone(),
            api_key,
        ));
        let _ = API_CLIENT_HANDLE.set(api_client.clone());
        let sync_index = Arc::new(Mutex::new(SyncIndex::new()));

        let qm = Arc::new(QueueManager::new(
            api_client,
            3,
            shared_state_startup.clone(),
            sync_index.clone(),
        ));

        // Start the live filesystem watcher immediately.
        let (tx, mut rx) = mpsc::channel(32);
        let watch_paths = config.watch_path_strings();
        let monitor = Monitor::new(watch_paths);
        monitor.start(tx);
        log::info!("File monitor started");

        // Feed monitor events into the upload queue, preserving per-path album config
        let qm_clone = qm.clone();
        let path_configs: Vec<_> = config.data.watch_paths.clone();
        tokio::spawn(async move {
            while let Some((path, checksum)) = rx.recv().await {
                log::info!("Queuing: {} (sha1={})", path, checksum);

                let mut album_id = None;
                let mut album_name = None;
                for entry in &path_configs {
                    use config::WatchPathEntry;
                    if let WatchPathEntry::WithConfig {
                        path: base,
                        album_id: aid,
                        album_name: aname,
                    } = entry
                    {
                        if path.starts_with(base.as_str()) {
                            album_id = aid.clone();
                            album_name = aname.clone();
                            break;
                        }
                    }
                }

                let _ = qm_clone
                    .add_to_queue(FileTask {
                        path,
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

        // Retain the queue manager so the shutdown path can flush retries to disk.
        let _ = QM_HANDLE.set(qm);

        // The startup scan backfills anything that arrived while Mimick was not running.
        tokio::spawn(async move {
            let startup_api = API_CLIENT_HANDLE
                .get()
                .cloned()
                .expect("API client should be initialized before startup scan");
            queue_unsynced_files(startup_paths, startup_qm, startup_sync_index, startup_api).await;
        });

        let app_clone2 = app.clone();
        let app_clone3 = app.clone();
        let shared_state2 = shared_state_startup.clone();

        // Cross-thread flag: Tokio sets it; the GTK timer reads and clears it.
        // Arc<Mutex<bool>> is Send + Sync, so it can cross the tokio::spawn boundary.
        let settings_flag = Arc::new(std::sync::Mutex::new(false));
        let settings_flag_writer = settings_flag.clone(); // moves into tokio::spawn (Send ✓)
        let quit_flag = Arc::new(std::sync::Mutex::new(false));
        let quit_flag_writer = quit_flag.clone(); // moves into tokio::spawn (Send ✓)

        // GTK-side: poll the flag every 250ms on the main thread.
        // app_clone2 / shared_state2 are !Send — they stay here, never enter spawns.
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
                let client = API_CLIENT_HANDLE.get().cloned();
                open_settings_if_needed(&app_clone2, shared_state2.clone(), client);
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

            glib::ControlFlow::Continue
        });

        // Tokio-side: build the tray and forward watch signals into the flag.
        // Only *_writer flags (Send ✓) and watch receivers (Send ✓) are captured here.
        tokio::spawn(async move {
            log::info!("Starting system tray");
            match build_tray().await {
                Ok((_handle, mut settings_rx, mut quit_rx)) => loop {
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
                    }
                },
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

        let open_settings = argv.contains(&"--settings".to_string())
            // Also open settings when activated by a secondary instance (e.g. clicking
            // the app icon in the launcher while the daemon is already running).
            || cmdline.is_remote();

        if open_settings {
            let client = API_CLIENT_HANDLE.get().cloned();
            open_settings_if_needed(app, shared_state_cmdline.clone(), client);
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
        if let Some(qm) = QM_HANDLE.get() {
            qm.flush_retries();
        }
        let state = shared_state.lock().unwrap().clone();
        StateManager::new().write_state(state);
        log::info!("Mimick exiting");
    }

    if take_restart_request() {
        if let Err(err) = launch_replacement(true) {
            log::error!("{err}");
        }
    }
}

/// Open the settings window only if one is not already visible.
fn open_settings_if_needed(
    app: &adw::Application,
    shared_state: Arc<Mutex<AppState>>,
    api_client: Option<Arc<ImmichApiClient>>,
) {
    if let Some(win) = app.windows().first() {
        win.present();
    } else {
        log::debug!("Opening settings window");
        build_settings_window(app, shared_state, api_client);
    }
}
