//! Handles application bootstrap, single-instance wiring, and daemon startup flow.
//!
//! Initialises GTK/Libadwaita, registers the D-Bus application name for
//! single-instance enforcement, and decides whether to present the library
//! window or settings window based on user configuration. Background sync,
//! tray icon, and filesystem monitor are wired up before entering the
//! main event loop.

use gtk::prelude::*;
use libadwaita as adw;
use log::Record;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

mod api_client;
mod app_context;
mod autostart;
mod cache_manager;
mod config;
mod diagnostics;
mod library;
mod media_kinds;
mod monitor;
mod notifications;
mod profile;
mod queue_manager;
mod remote_sync;
mod runtime_env;
mod sanitize;
mod settings_window;
mod sidecar;
mod startup_scan;
mod state_manager;
mod sync_index;
mod tray_icon;
mod util;
mod watch_path_display;

use api_client::{ImmichApiClient, LibraryAsset};
use app_context::AppContext;
use config::{Config, best_matching_watch_entry};
use library::state::LibraryState;
use library::thumbnail_cache::ThumbnailCache;
use monitor::{Monitor, MonitorEvent};
use queue_manager::{EnvironmentPolicy, FileTask, QueueManager};
use settings_window::build_settings_window;
use startup_scan::queue_unsynced_files;
use state_manager::{AppState, StateManager};
use sync_index::{ShardedSyncIndex, SyncDecision, SyncTarget};
use tray_icon::build_tray;

use flexi_logger::{
    Cleanup, Criterion, DeferredNow, Duplicate, FileSpec, Logger, Naming, WriteMode, style,
};
use std::io::Write;

/// Shared application context reused by UI entry points and the shutdown path.
static APP_CONTEXT: std::sync::OnceLock<Arc<AppContext>> = std::sync::OnceLock::new();

/// Description of a local deletion event request targeting the remote album.
#[derive(Clone, Debug)]
struct LocalDeletionRequest {
    /// Absolute local path of deleted asset.
    local_path: String,
    /// Target Immich asset identifier.
    asset_id: String,
    /// Absolute filename of deleted asset.
    asset_name: String,
    /// Mapped Immich album name.
    album_name: String,
    /// Mapped Immich album identifier.
    album_id: Option<String>,
}

/// Helper to extract formatted filename and line number from logs.
fn format_log_location(record: &Record) -> String {
    match (record.file(), record.line()) {
        (Some(file), Some(line)) => format!(" {}:{}", file, line),
        _ => String::new(),
    }
}

/// Logger formatter that produces plain text for files.
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

/// Logger formatter that produces ANSI color output for terminal displays.
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

/// Check if local deletion matches validation criteria for remote mirror sweep.
async fn build_local_deletion_request(
    ctx: Arc<AppContext>,
    path: String,
) -> Option<LocalDeletionRequest> {
    let record = match ctx.sync_index.record_for_path(&path) {
        Some(record) => record,
        None => {
            log::debug!("No sync record for deleted file: {}", path);
            return None;
        }
    };

    let path_obj = std::path::Path::new(&path);
    let entry = {
        let entries = ctx.live_watch_paths.lock();
        best_matching_watch_entry(path_obj, &entries).cloned()
    };
    let Some(entry) = entry else {
        log::debug!("Deleted file is not under any watch folder: {}", path);
        return None;
    };
    let rules = entry.rules();
    if !rules.delete_folder_to_album {
        log::debug!("Folder-to-album deletion disabled for: {}", path);
        return None;
    }

    let album_name = entry
        .album_name()
        .map(|name| name.to_string())
        .or(record.album_name.clone())
        .or_else(|| {
            path_obj
                .parent()
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "Mimick".to_string());

    let configured_album_id = match &entry {
        config::WatchPathEntry::WithConfig { album_id, .. } => album_id.clone(),
        config::WatchPathEntry::Simple(_) => None,
    };
    let album_id = match configured_album_id.or(record.album_id.clone()) {
        Some(id) => Some(id),
        None => match ctx.api_client.get_album_id_if_exists(&album_name).await {
            Ok(id) => id,
            Err(err) => {
                log::warn!(
                    "Could not resolve album '{}' for deletion sync: {}",
                    album_name,
                    err
                );
                None
            }
        },
    }?;

    match find_album_asset_by_checksum(ctx.api_client.clone(), &album_id, &record.checksum).await {
        Ok(Some(asset)) => Some(LocalDeletionRequest {
            local_path: path,
            asset_id: asset.id,
            asset_name: asset.filename,
            album_name,
            album_id: Some(album_id),
        }),
        Ok(None) => {
            log::debug!("No matching album asset for deleted file: {}", path);
            None
        }
        Err(err) => {
            log::warn!("Could not inspect album for deletion sync: {}", err);
            None
        }
    }
}

/// Mirrors local filesystem deletion by unlinking or trashing assets on Immich.
async fn trash_remote_after_local_delete(ctx: Arc<AppContext>, request: LocalDeletionRequest) {
    let asset_ids = vec![request.asset_id.clone()];
    let album_count = ctx
        .api_client
        .count_albums_for_asset(&request.asset_id)
        .await;
    let (succeeded, action_log) = match (album_count, request.album_id.as_deref()) {
        (Some(n), Some(album_id)) if n > 1 => {
            let ok = ctx
                .api_client
                .remove_assets_from_album(album_id, &asset_ids)
                .await;
            (
                ok,
                format!(
                    "Unlinked '{}' from album '{}' (asset belongs to {} albums; preserved on server)",
                    request.asset_name, request.album_name, n
                ),
            )
        }
        _ => match ctx.api_client.delete_assets(&asset_ids).await {
            Ok(()) => (
                true,
                format!(
                    "Mirrored local delete of '{}' to album '{}' (asset trashed on server)",
                    request.asset_name, request.album_name
                ),
            ),
            Err(err) => {
                log::warn!(
                    "Could not mirror local delete of '{}': {}; sync record kept for retry",
                    request.asset_name,
                    err
                );
                return;
            }
        },
    };
    if !succeeded {
        log::warn!(
            "Could not mirror local delete of '{}'; sync record kept for retry",
            request.asset_name
        );
        return;
    }
    if let Err(err) = ctx.sync_index.remove_path(&request.local_path) {
        log::warn!(
            "Server-side delete succeeded but sync record cleanup failed for '{}': {}",
            request.local_path,
            err
        );
    }
    log::info!("{}", action_log);
}

/// Iterate through album assets matching checksum to find matching Immich library record.
async fn find_album_asset_by_checksum(
    api_client: Arc<ImmichApiClient>,
    album_id: &str,
    checksum: &str,
) -> Result<Option<LibraryAsset>, String> {
    let mut page = 1;
    loop {
        let (assets, has_more) = api_client
            .fetch_album_assets(album_id, page, 1000, None)
            .await?;
        if let Some(asset) = assets
            .into_iter()
            .find(|asset| asset.checksum.as_deref() == Some(checksum))
        {
            return Ok(Some(asset));
        }
        if !has_more {
            return Ok(None);
        }
        page += 1;
    }
}

/// Suppress stderr noise from `rawloader`/`imagepipe` panics — they're
/// caught via `catch_unwind` at the call site, but the default hook still
/// prints before unwinding reaches it.
fn install_filtering_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let file = info.location().map(|l| l.file()).unwrap_or("");
        if file.contains("/rawloader-") || file.contains("/imagepipe-") {
            return;
        }
        default(info);
    }));
}

#[tokio::main]
async fn main() {
    // Mirror logs to stdout and to a rotating cache file for easier support/debugging.
    let log_dir = profile::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp").join(profile::dir_segment()));

    // Named profiles (e.g. MIMICK_PROFILE=dev) default to verbose mimick logs
    let default_log_spec = if profile::name().is_some() {
        "mimick=debug,info"
    } else {
        "info"
    };
    let _logger = Logger::try_with_env_or_str(default_log_spec)
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

    install_filtering_panic_hook();

    if let Some(name) = profile::name() {
        log::info!(
            "Active profile: {} (state dirs use segment '{}')",
            name,
            profile::dir_segment()
        );
    }

    gtk::gio::resources_register_include!("mimick.gresource")
        .expect("Failed to register bundled GResource");

    let app = adw::Application::builder()
        .application_id(profile::application_id())
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let is_primary_instance = Arc::new(AtomicBool::new(false));
    let is_primary_instance_clone = is_primary_instance.clone();

    let shared_state: Arc<parking_lot::Mutex<AppState>> = Arc::new(parking_lot::Mutex::new({
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
        saved.reset_runtime_state();

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
            let mut state = shared_state_startup.lock();
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
        let sync_index = Arc::new(ShardedSyncIndex::new());

        let sync_index_flusher = sync_index.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                let _ = sync_index_flusher.flush();
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

        // Sync the RAW decode cache flag with the user's persisted preference.
        crate::library::set_raw_cache_enabled(config.data.raw_decode_cache_enabled);
        crate::library::set_raw_full_decode(config.data.raw_full_decode);

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

        let startup_qm = qm.clone();
        let startup_paths = config.data.watch_paths.clone();
        let startup_sync_index = sync_index.clone();
        let (manual_sync_tx, mut manual_sync_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let thumbnail_cache = Arc::new(ThumbnailCache::new(api_client.clone()));
        // One-shot startup prune across every cache directory. Runs on a
        // blocking thread after a short delay so it does not contend with
        // window setup or initial sync work.
        let cache_cap_mb = config.data.cache_disk_cap_mb;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let cap_bytes = (cache_cap_mb as u64).saturating_mul(1024 * 1024);
            let _ = tokio::task::spawn_blocking(move || {
                cache_manager::prune_all_blocking(cap_bytes);
            })
            .await;
        });
        let library_state = Arc::new(parking_lot::Mutex::new(LibraryState::new()));
        let shared_config = Arc::new(parking_lot::RwLock::new(config));
        let ctx = Arc::new(AppContext {
            config: shared_config.clone(),
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
            current_user_id: Arc::new(parking_lot::Mutex::new(None)),
            expected_self_deletions: Arc::new(app_context::RecentSelfPaths::default()),
            expected_self_downloads: Arc::new(app_context::RecentSelfPaths::default()),
            reconcile_locks: Arc::new(app_context::ReconcileLocks::default()),
            pending_deletions: Arc::new(app_context::PendingDeletions::default()),
        });
        let _ = APP_CONTEXT.set(ctx.clone());

        let qm_clone = qm.clone();
        let live_watch_paths_for_queue = live_watch_paths.clone();
        let deletion_ctx = ctx.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    MonitorEvent::Ready { path, checksum, sidecar_path } => {
                        if deletion_ctx.expected_self_downloads.consume(&path) {
                            continue;
                        }

                        let (album_id, album_name, watch_path, folder_rules) = {
                            let path_configs = live_watch_paths_for_queue.lock();
                            best_matching_watch_entry(std::path::Path::new(&path), &path_configs)
                                .map(|entry| match entry {
                                    config::WatchPathEntry::WithConfig {
                                        album_id,
                                        album_name,
                                        rules,
                                        ..
                                    } => (
                                        album_id.clone(),
                                        album_name.clone(),
                                        entry.path().to_string(),
                                        rules.clone(),
                                    ),
                                    config::WatchPathEntry::Simple(_) => {
                                        (None, None, entry.path().to_string(), Default::default())
                                    }
                                })
                                .unwrap_or((None, None, String::new(), Default::default()))
                        };

                        // Resolve XMP: per-folder override -> global default.
                        let global_xmp = deletion_ctx.config.read().data.upload_xmp_sidecars;
                        let sidecar_path = if folder_rules.xmp_sidecar_enabled(global_xmp) {
                            sidecar_path
                        } else {
                            None
                        };

                        let target = SyncTarget {
                            album_name: album_name.clone(),
                            album_id: album_id.clone(),
                        };
                        let (reassociate_only, task_checksum) = match deletion_ctx
                            .sync_index
                            .sync_decision(std::path::Path::new(&path), &target)
                        {
                            Ok(SyncDecision::UpToDate) => {
                                log::debug!("Skipping unchanged file: {}", path);
                                continue;
                            }
                            Ok(SyncDecision::NeedsReassociate) => (
                                true,
                                deletion_ctx
                                    .sync_index
                                    .stored_checksum(&path)
                                    .unwrap_or(checksum),
                            ),
                            Ok(SyncDecision::NeedsUpload) => (false, checksum),
                            Err(err) => {
                                log::warn!(
                                    "Could not inspect sync index for '{}': {}; queuing anyway",
                                    path,
                                    err
                                );
                                (false, checksum)
                            }
                        };

                        log::info!("Queuing: {} (sha1={})", path, task_checksum);

                        let _ = qm_clone
                            .add_to_queue(FileTask {
                                path,
                                watch_path,
                                checksum: task_checksum,
                                album_id,
                                album_name,
                                reassociate_only,
                                skip_album: false,
                                sidecar_path,
                            })
                            .await;
                    }
                    MonitorEvent::Deleted { path } => {
                        if deletion_ctx.expected_self_deletions.consume(&path) {
                            continue;
                        }
                        if let Some(request) =
                            build_local_deletion_request(deletion_ctx.clone(), path).await
                        {
                            trash_remote_after_local_delete(deletion_ctx.clone(), request).await;
                        }
                    }
                }
            }
        });

        // The startup scan backfills anything that arrived while Mimick was not running.
        if background_sync_enabled {
            let shared_state_startup_task = shared_state_startup.clone();
            let startup_api = ctx.api_client.clone();
            let startup_ctx = ctx.clone();
            let catchup_mode = shared_config.read().data.startup_catchup_mode.clone();
            tokio::spawn(async move {
                queue_unsynced_files(
                    startup_paths,
                    startup_qm,
                    startup_sync_index,
                    startup_api,
                    catchup_mode,
                    shared_state_startup_task,
                    startup_ctx,
                )
                .await;
            });
        } else {
            log::info!("Background sync is disabled; skipping startup catch-up scan");
        }

        if background_sync_enabled {
            let reconciler_ctx = ctx.clone();
            tokio::spawn(async move {
                remote_sync::run_album_reconciler(reconciler_ctx).await;
            });
        }

        let startup_state = shared_state_startup.clone();
        let status_api = ctx.api_client.clone();
        tokio::spawn(async move {
            let connected = status_api.check_connection().await;
            let route = status_api.active_route_label().await;
            let latest_issue = status_api.latest_issue().await;

            let mut state = startup_state.lock();
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
        let manual_config = shared_config.clone();
        let manual_ctx = ctx.clone();
        tokio::spawn(async move {
            while manual_sync_rx.recv().await.is_some() {
                let (watch_paths, catchup_mode) = {
                    let cfg = manual_config.read();
                    (cfg.data.watch_paths.clone(), cfg.data.startup_catchup_mode.clone())
                };
                queue_unsynced_files(
                    watch_paths,
                    manual_qm.clone(),
                    manual_sync_index.clone(),
                    manual_sync_api.clone(),
                    catchup_mode,
                    shared_state_manual_task.clone(),
                    manual_ctx.clone(),
                )
                .await;
            }
        });

        let app_clone2 = app.clone();
        let app_clone3 = app.clone();

        // Cross-thread flag: Tokio sets it; the GTK timer reads and clears it.
        // Arc<Mutex<bool>> is Send + Sync, so it can cross the tokio::spawn boundary.
        let settings_flag = Arc::new(parking_lot::Mutex::new(false));
        let settings_flag_writer = settings_flag.clone(); // moves into tokio::spawn (Send ✓)
        let library_flag = Arc::new(parking_lot::Mutex::new(false));
        let library_flag_writer = library_flag.clone();
        let quit_flag = Arc::new(parking_lot::Mutex::new(false));
        let quit_flag_writer = quit_flag.clone(); // moves into tokio::spawn (Send ✓)
        let pause_flag = Arc::new(parking_lot::Mutex::new(false));
        let pause_flag_writer = pause_flag.clone();
        let sync_now_flag = Arc::new(parking_lot::Mutex::new(false));
        let sync_now_flag_writer = sync_now_flag.clone();

        // GTK-side: poll the flag every 250ms on the main thread.
        // The application handle stays on the GTK thread and never enters Tokio tasks.
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let settings_triggered = {
                let mut f = settings_flag.lock();
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
                open_settings_window_now(&app_clone2, ctx);
            }

            let library_triggered = {
                let mut f = library_flag.lock();
                if *f {
                    *f = false;
                    true
                } else {
                    false
                }
            };
            if library_triggered {
                let ctx = APP_CONTEXT
                    .get()
                    .cloned()
                    .expect("App context should be initialized before opening library");
                open_library_window_now(&app_clone2, ctx);
            }

            let quit_triggered = {
                let mut f = quit_flag.lock();
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
                let mut f = pause_flag.lock();
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
                let mut f = sync_now_flag.lock();
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
        let tray_library_enabled = shared_config.read().data.library_view_enabled;
        tokio::spawn(async move {
            log::info!("Starting system tray");
            match build_tray(tray_library_enabled).await {
                Ok(handles) => {
                    let crate::tray_icon::TrayHandles {
                        handle: _handle,
                        mut settings_rx,
                        mut library_rx,
                        mut quit_rx,
                        mut pause_rx,
                        mut sync_now_rx,
                    } = handles;
                    loop {
                        tokio::select! {
                            res = settings_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *settings_rx.borrow() {
                                    *settings_flag_writer.lock() = true;
                                }
                            }
                            res = library_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *library_rx.borrow() {
                                    *library_flag_writer.lock() = true;
                                }
                            }
                            res = quit_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *quit_rx.borrow() {
                                    *quit_flag_writer.lock() = true;
                                }
                            }
                            res = pause_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *pause_rx.borrow() {
                                    *pause_flag_writer.lock() = true;
                                }
                            }
                            res = sync_now_rx.changed() => {
                                if res.is_err() {
                                    break;
                                }
                                if *sync_now_rx.borrow() {
                                    *sync_now_flag_writer.lock() = true;
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

        let ctx_early = APP_CONTEXT.get().cloned();
        let want_settings = argv.contains(&"--settings".to_string());
        let want_library = argv.contains(&"--library".to_string());
        let setup_required = ctx_early
            .as_ref()
            .map(|c| c.config.read().get_api_key().unwrap_or_default().is_empty())
            .unwrap_or(true);
        let secondary_activation = cmdline.is_remote();

        let ctx_lookup = || {
            APP_CONTEXT
                .get()
                .cloned()
                .expect("App context should be initialized before command-line activation")
        };

        if want_settings || setup_required {
            open_settings_window_now(app, ctx_lookup());
        } else if want_library {
            open_library_window_now(app, ctx_lookup());
        } else if secondary_activation
            || !ctx_early
                .as_ref()
                .map(|c| c.config.read().data.background_sync_enabled)
                .unwrap_or(false)
        {
            open_default_window(app, ctx_lookup());
        }

        app.activate();
        0.into()
    });

    app.connect_activate(move |_app| {
        log::debug!("App activated");
    });

    log::info!("GTK application starting up");

    if is_primary_instance.load(Ordering::SeqCst) {
        let quit_requested = Arc::new(AtomicBool::new(false));
        let qr_signal = quit_requested.clone();
        tokio::spawn(async move {
            let mut sigterm =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(err) => {
                        log::warn!("Could not install SIGTERM handler: {}", err);
                        return;
                    }
                };
            tokio::select! {
                res = tokio::signal::ctrl_c() => {
                    if let Err(err) = res {
                        log::warn!("SIGINT handler error: {}", err);
                        return;
                    }
                    log::info!("Received SIGINT; requesting graceful shutdown.");
                }
                _ = sigterm.recv() => {
                    log::info!("Received SIGTERM; requesting graceful shutdown.");
                }
            }
            qr_signal.store(true, Ordering::SeqCst);
        });

        let app_for_quit = app.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if quit_requested.load(Ordering::SeqCst) {
                app_for_quit.quit();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    app.run();

    // Persist final state and any pending retries on graceful shutdown.
    if is_primary_instance.load(Ordering::SeqCst) {
        if let Some(ctx) = APP_CONTEXT.get() {
            ctx.queue_manager
                .shutdown(std::time::Duration::from_secs(5))
                .await;
            ctx.queue_manager.flush_retries();
            if let Err(err) = ctx.sync_index.flush() {
                log::warn!("Failed to flush sync index on shutdown: {}", err);
            }
        }
        let state = APP_CONTEXT
            .get()
            .map(|ctx| ctx.state.lock().clone())
            .unwrap_or_else(|| shared_state.lock().clone());
        StateManager::new().write_state(state);
        log::info!("Mimick exiting");
    }
}

/// Open whichever window the user prefers, presenting an existing instance if available.
fn open_default_window(app: &adw::Application, ctx: Arc<AppContext>) {
    if let Some(win) = find_window(app, "mimick-library-window")
        .or_else(|| find_window(app, "mimick-settings-window"))
    {
        win.present();
        return;
    }
    if ctx.config.read().data.library_view_enabled {
        open_library_window_now(app, ctx);
    } else {
        open_settings_window_now(app, ctx);
    }
}

/// Open Settings window or present existing settings instance.
fn open_settings_window_now(app: &adw::Application, ctx: Arc<AppContext>) {
    if let Some(win) = find_window(app, "mimick-settings-window") {
        win.present();
        return;
    }
    log::debug!("Opening settings window");
    build_settings_window(app, ctx);
}

/// Open Library window, falling back to Settings if disabled.
fn open_library_window_now(app: &adw::Application, ctx: Arc<AppContext>) {
    if let Some(win) = find_window(app, "mimick-library-window") {
        win.present();
        return;
    }
    if !ctx.config.read().data.library_view_enabled {
        log::info!("Library view is disabled in settings; opening Settings instead");
        open_settings_window_now(app, ctx);
        return;
    }
    log::debug!("Opening library window");
    library::build_library_window(app, ctx);
}

/// Helper to look up active GTK window instances by widget name.
fn find_window(app: &adw::Application, name: &str) -> Option<gtk::Window> {
    app.windows().into_iter().find(|w| w.widget_name() == name)
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
