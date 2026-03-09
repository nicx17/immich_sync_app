use gtk::prelude::*;
use libadwaita as adw;
use tokio::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

mod config;
mod api_client;
mod state_manager;
mod queue_manager;
mod monitor;
mod notifications;
mod settings_window;
mod tray_icon;

use config::Config;
use api_client::ImmichApiClient;
use queue_manager::{QueueManager, FileTask};
use monitor::Monitor;
use settings_window::build_settings_window;
use tray_icon::build_tray;
use state_manager::{StateManager, AppState};

/// Holds the primary instance's QueueManager so the shutdown path can flush retries to disk.
static QM_HANDLE: std::sync::OnceLock<Arc<QueueManager>> = std::sync::OnceLock::new();
/// Shared API client — created once, reused by the settings window on every open.
static API_CLIENT_HANDLE: std::sync::OnceLock<Arc<ImmichApiClient>> = std::sync::OnceLock::new();

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();

    let app = adw::Application::builder()
        .application_id("com.nickcardoso.mimick")
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let is_primary_instance = Arc::new(AtomicBool::new(false));
    let is_primary_instance_clone = is_primary_instance.clone();

    // Shared in-memory state — workers write directly, UI reads directly.
    // No disk I/O during normal operation; disk is only read on startup (crash recovery)
    // and written on graceful shutdown.
    let shared_state: Arc<Mutex<AppState>> = Arc::new(Mutex::new({
        let saved = StateManager::new().read_state();
        // Reset volatile fields that shouldn't survive a restart
        AppState { status: "idle".to_string(), active_workers: 0, ..saved }
    }));

    let shared_state_startup = shared_state.clone();
    let shared_state_cmdline = shared_state.clone();

    // The startup signal runs ONLY in the primary instance once it has successfully
    // acquired the D-Bus name. We initialize the daemon processes here to prevent
    // secondary instances from spawning dummy trays and file monitors before exiting.
    app.connect_startup(move |app| {
        is_primary_instance_clone.store(true, Ordering::SeqCst);

        log::info!("Mimick primary instance initializing");

        // Keep the app alive even when no windows are open (daemon / tray mode).
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

        let qm = Arc::new(QueueManager::new(api_client, 3, shared_state_startup.clone()));

        // Start file monitor using plain path strings
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
                    if let WatchPathEntry::WithConfig { path: base, album_id: aid, album_name: aname } = entry {
                        if path.starts_with(base.as_str()) {
                            album_id = aid.clone();
                            album_name = aname.clone();
                            break;
                        }
                    }
                }

                qm_clone.add_to_queue(FileTask { path, checksum, album_id, album_name }).await;
            }
        });

        // Store in the global handle so main() can call flush_retries() on graceful shutdown.
        let _ = QM_HANDLE.set(qm);

        let app_clone2 = app.clone();
        let shared_state2 = shared_state_startup.clone();

        // Cross-thread flag: Tokio sets it; the GTK timer reads and clears it.
        // Arc<Mutex<bool>> is Send + Sync, so it can cross the tokio::spawn boundary.
        let flag = Arc::new(std::sync::Mutex::new(false));
        let flag_writer = flag.clone(); // moves into tokio::spawn (Send ✓)

        // GTK-side: poll the flag every 250ms on the main thread.
        // app_clone2 / shared_state2 are !Send — they stay here, never enter spawns.
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let triggered = {
                let mut f = flag.lock().unwrap();
                if *f { *f = false; true } else { false }
            };
            if triggered {
                let client = API_CLIENT_HANDLE.get().cloned();
                open_settings_if_needed(&app_clone2, shared_state2.clone(), client);
            }
            glib::ControlFlow::Continue
        });

        // Tokio-side: build the tray and forward watch signals into the flag.
        // Only flag_writer (Send ✓) and settings_rx (Send ✓) are captured here.
        tokio::spawn(async move {
            log::info!("Starting system tray");
            match build_tray().await {
                Ok((_handle, mut settings_rx)) => {
                    while settings_rx.changed().await.is_ok() {
                        if *settings_rx.borrow() {
                            *flag_writer.lock().unwrap() = true;
                        }
                    }
                }
                Err(e) => log::warn!("System tray failed to start: {:?}", e),
            }
        });
    });

    // Handle command line from both the primary and secondary instances.
    app.connect_command_line(move |app, cmdline| {
        let argv: Vec<String> = cmdline.arguments()
            .iter()
            .filter_map(|a| a.to_str().map(|s| s.to_string()))
            .collect();

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
}

/// Open the settings window only if one is not already visible.
fn open_settings_if_needed(app: &adw::Application, shared_state: Arc<Mutex<AppState>>, api_client: Option<Arc<ImmichApiClient>>) {
    if let Some(win) = app.windows().first() {
        win.present();
    } else {
        log::debug!("Opening settings window");
        build_settings_window(app, shared_state, api_client);
    }
}
