//! Download flow, video handoff, and transfer-tracking helpers.
//!
//! Streams the original-quality asset from the server into a local file,
//! updating the progress bar and transfer rate display. Video files can
//! optionally be handed off to the system default player after download.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::TransferProgressCallback;
use crate::app_context::AppContext;
use crate::library::state::LibrarySource;
use crate::state_manager::TransferDirection;

use super::LibraryWindowUi;

pub(super) fn begin_download_session(ctx: &Arc<AppContext>, item_label: String) {
    let state_ref = ctx.state.clone();
    let mut state = state_ref.lock();
    let route = state.active_server_route.clone();
    state
        .transfer
        .begin_group(TransferDirection::Download, Some(item_label), route);
}

pub(super) fn track_download_item(
    ctx: &Arc<AppContext>,
    item_id: String,
    item_label: Option<String>,
    total_bytes: Option<u64>,
) -> TransferProgressCallback {
    let state_ref = ctx.state.clone();
    {
        let mut state = state_ref.lock();
        let route = state.active_server_route.clone();
        state.transfer.register_item(
            TransferDirection::Download,
            item_id.clone(),
            total_bytes,
            item_label,
            route,
        );
    }
    Arc::new(move |bytes_done, total_bytes| {
        let mut state = state_ref.lock();
        if let Some(total_bytes) = total_bytes {
            let current = state
                .transfer
                .active_item_totals
                .get(&item_id)
                .copied()
                .unwrap_or(0);
            if current == 0 {
                state.transfer.update_item_total(&item_id, total_bytes);
            }
        }
        let route = state.active_server_route.clone();
        state
            .transfer
            .update_item_bytes(TransferDirection::Download, &item_id, bytes_done, route);
    })
}

pub(super) fn finish_download_item(ctx: &Arc<AppContext>, item_id: &str) {
    let mut state = ctx.state.lock();
    let route = state.active_server_route.clone();
    state
        .transfer
        .finish_item(TransferDirection::Download, item_id, route);
}

/// Hand a local file off to the user's default app via `xdg-open`/equivalent.
/// Used for local videos per the spec — no in-app playback in v1.
pub(super) fn open_local_with_default_app(path: &str) {
    let uri = format!("file://{}", path);
    if let Err(err) =
        gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>)
    {
        log::warn!("Failed to open {}: {}", uri, err);
    }
}

pub(super) fn spawn_video_handoff(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String) {
    glib::MainContext::default().spawn_local(async move {
        let Some(cache_dir) = crate::profile::cache_dir().map(|p| p.join("video")) else {
            return;
        };
        let _ = std::fs::create_dir_all(&cache_dir);
        let safe_name =
            crate::sanitize::safe_filename(&filename).unwrap_or_else(|| asset_id.clone());
        let path = cache_dir.join(&safe_name);
        if !path.exists()
            && let Err(err) = {
                begin_download_session(&ui.ctx, filename.clone());
                let progress =
                    track_download_item(&ui.ctx, asset_id.clone(), Some(filename.clone()), None);
                let result = ui
                    .ctx
                    .api_client
                    .download_original_to_file(&asset_id, &path, Some(progress))
                    .await;
                finish_download_item(&ui.ctx, &asset_id);
                result
            }
        {
            log::warn!("Video handoff failed for {}: {}", asset_id, err);
            return;
        }
        open_local_with_default_app(&path.display().to_string());
    });
}

pub(super) fn start_download(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String) {
    begin_download_session(&ui.ctx, filename.clone());
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let Some(target_dir) = ensure_download_target(&ui).await else {
                return;
            };
            let safe_name =
                crate::sanitize::safe_filename(&filename).unwrap_or_else(|| asset_id.clone());
            let output_path = target_dir.join(&safe_name);
            if output_path.exists() && !show_overwrite_dialog(&ui, &safe_name).await {
                return;
            }
            match do_download(&ui, &asset_id, &output_path).await {
                Ok(()) => {
                    let alert = libadwaita::AlertDialog::builder()
                        .heading("Download Complete")
                        .body(format!("Saved {}", safe_name))
                        .build();
                    alert.add_response("ok", "OK");
                    alert.present(Some(&ui.window));
                }
                Err(err) => {
                    let alert = libadwaita::AlertDialog::builder()
                        .heading("Download Failed")
                        .body(&err)
                        .build();
                    alert.add_response("ok", "OK");
                    alert.present(Some(&ui.window));
                }
            }
        }
    ));
}

#[derive(Clone, Copy)]
enum ConflictAction {
    Skip,
    Overwrite,
    Rename,
}

pub(super) fn start_download_group(ui: Rc<LibraryWindowUi>, downloads: Vec<(String, String)>) {
    begin_download_session(&ui.ctx, format!("{} items", downloads.len()));
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let Some(target_dir) = ensure_download_target(&ui).await else {
                return;
            };

            let mut succeeded: u32 = 0;
            let mut failed: u32 = 0;
            let mut skipped: u32 = 0;
            let mut conflict_policy: Option<ConflictAction> = None;

            for (asset_id, filename) in &downloads {
                let safe_name =
                    crate::sanitize::safe_filename(filename).unwrap_or_else(|| asset_id.clone());
                let mut output_path = target_dir.join(&safe_name);

                if output_path.exists() {
                    match resolve_conflict(&ui, &safe_name, &mut conflict_policy).await {
                        ConflictAction::Skip => {
                            skipped += 1;
                            continue;
                        }
                        ConflictAction::Overwrite => {}
                        ConflictAction::Rename => {
                            output_path = unique_path(&target_dir, &safe_name);
                        }
                    }
                }

                match do_download(&ui, asset_id, &output_path).await {
                    Ok(()) => succeeded += 1,
                    Err(_) => failed += 1,
                }
            }

            ui.grid.selection.unselect_all();
            ui.select_toggle.set_active(false);
            show_batch_summary(&ui, succeeded, failed, skipped, &target_dir);
        }
    ));
}

async fn resolve_conflict(
    ui: &LibraryWindowUi,
    filename: &str,
    policy: &mut Option<ConflictAction>,
) -> ConflictAction {
    if let Some(a) = *policy {
        return a;
    }
    let (action, apply_all) = show_batch_conflict_dialog(ui, filename).await;
    if apply_all {
        *policy = Some(action);
    }
    action
}

fn show_batch_summary(
    ui: &LibraryWindowUi,
    succeeded: u32,
    failed: u32,
    skipped: u32,
    target_dir: &Path,
) {
    let folder_name = folder_display_name(target_dir);
    let (heading, body) = if succeeded == 0 && failed == 0 {
        (
            "No Assets Downloaded",
            format!("All {} asset(s) were skipped", skipped),
        )
    } else if failed == 0 && skipped == 0 {
        (
            "Download Complete",
            format!("Downloaded {} asset(s) to {}", succeeded, folder_name),
        )
    } else {
        let mut parts = vec![format!(
            "Downloaded {} asset(s) to {}",
            succeeded, folder_name
        )];
        if skipped > 0 {
            parts.push(format!("{} skipped", skipped));
        }
        if failed > 0 {
            parts.push(format!("{} failed", failed));
        }
        (
            if succeeded > 0 {
                "Download Complete"
            } else {
                "Download Failed"
            },
            parts.join("\n"),
        )
    };
    let alert = libadwaita::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    alert.add_response("ok", "OK");
    alert.present(Some(&ui.window));
}

async fn do_download(
    ui: &Rc<LibraryWindowUi>,
    asset_id: &str,
    output_path: &Path,
) -> Result<(), String> {
    let item_label = output_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| output_path.display().to_string());
    let progress = track_download_item(&ui.ctx, asset_id.to_string(), Some(item_label), None);
    let result = ui
        .ctx
        .api_client
        .download_original_to_file(asset_id, output_path, Some(progress))
        .await;
    finish_download_item(&ui.ctx, asset_id);
    if result.is_ok() {
        let session_finished = !ui.ctx.state.lock().transfer.active;
        if should_refresh_after_download(ui) && session_finished {
            super::refresh_library_after_mutation(ui.clone(), true);
        }
    }
    result
}

async fn show_overwrite_dialog(ui: &LibraryWindowUi, filename: &str) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::cell::Cell::new(Some(tx));
    let dialog = libadwaita::AlertDialog::builder()
        .heading("File already exists")
        .body(format!("\"{}\" already exists. Overwrite?", filename))
        .build();
    dialog.add_response("skip", "Skip");
    dialog.add_response("overwrite", "Overwrite");
    dialog.set_response_appearance("overwrite", libadwaita::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, response| {
        if let Some(tx) = tx.take() {
            let _ = tx.send(response == "overwrite");
        }
    });
    dialog.present(Some(&ui.window));
    rx.await.unwrap_or(false)
}

async fn show_batch_conflict_dialog(
    ui: &LibraryWindowUi,
    filename: &str,
) -> (ConflictAction, bool) {
    let (tx, rx) = tokio::sync::oneshot::channel::<(ConflictAction, bool)>();
    let tx = std::cell::Cell::new(Some(tx));
    let dialog = libadwaita::AlertDialog::builder()
        .heading("File already exists")
        .body(format!(
            "\"{}\" already exists in the download folder.",
            filename
        ))
        .build();
    let apply_all = gtk::CheckButton::builder()
        .label("Apply to all remaining conflicts")
        .build();
    dialog.set_extra_child(Some(&apply_all));
    dialog.add_response("skip", "Skip");
    dialog.add_response("rename", "Rename");
    dialog.add_response("overwrite", "Overwrite");
    dialog.set_response_appearance("overwrite", libadwaita::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("rename"));
    dialog.connect_response(
        None,
        clone!(
            #[weak]
            apply_all,
            move |_, response| {
                let all = apply_all.is_active();
                let action = match response {
                    "rename" => ConflictAction::Rename,
                    "overwrite" => ConflictAction::Overwrite,
                    _ => ConflictAction::Skip,
                };
                if let Some(tx) = tx.take() {
                    let _ = tx.send((action, all));
                }
            }
        ),
    );
    dialog.present(Some(&ui.window));
    rx.await.unwrap_or((ConflictAction::Skip, false))
}

fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let name = Path::new(filename);
    let stem = name
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = name.extension().and_then(|e| e.to_str());
    for i in 1..1000 {
        let candidate = match ext {
            Some(e) => dir.join(format!("{} ({}).{}", stem, i, e)),
            None => dir.join(format!("{} ({})", stem, i)),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}_copy", filename))
}

fn folder_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("selected folder")
        .to_string()
}

pub(super) async fn ensure_download_target(ui: &LibraryWindowUi) -> Option<PathBuf> {
    if let Some(path) = ui.ctx.config.read().data.download_target_path.clone() {
        return Some(PathBuf::from(path));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = gtk::FileDialog::builder()
        .title("Choose Download Folder")
        .build();
    dialog.select_folder(Some(&ui.window), gtk::gio::Cancellable::NONE, move |res| {
        let _ = tx.send(
            res.ok()
                .and_then(|folder| folder.path())
                .map(|path| path.to_path_buf()),
        );
    });
    let path = rx.await.ok().flatten()?;

    let (save_tx, save_rx) = tokio::sync::oneshot::channel::<bool>();
    let save_tx = std::cell::Cell::new(Some(save_tx));
    let confirm = libadwaita::AlertDialog::builder()
        .heading("Save as default?")
        .body(format!("Always download to {}?", path.display()))
        .build();
    confirm.add_response("once", "Just Once");
    confirm.add_response("always", "Always");
    confirm.set_default_response(Some("always"));
    confirm.connect_response(None, move |dlg, response| {
        if let Some(tx) = save_tx.take() {
            let _ = tx.send(response == "always");
        }
        dlg.close();
    });
    confirm.present(Some(&ui.window));

    if save_rx.await.unwrap_or(false) {
        let mut config = ui.ctx.config.write();
        config.data.download_target_path = Some(path.to_string_lossy().to_string());
        let _ = config.save();
    }
    Some(path)
}

pub(super) fn should_refresh_after_download(ui: &LibraryWindowUi) -> bool {
    matches!(
        ui.ctx.library_state.lock().source,
        LibrarySource::LocalAll
            | LibrarySource::LocalSearch { .. }
            | LibrarySource::Unified
            | LibrarySource::UnifiedSearch { .. }
            | LibrarySource::AlbumLocal { .. }
            | LibrarySource::AlbumUnified { .. }
    )
}

pub(super) fn format_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec.max(0.0))
    }
}
