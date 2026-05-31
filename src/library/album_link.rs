//! Album-folder linking, sync dialog, and link/unlink actions.
//!
//! Presents a dialog to associate a local watch folder with a remote
//! Immich album for bidirectional synchronization. Handles link state
//! persistence and triggers the initial diff when a new link is created.

use std::rc::Rc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::library::state::LibrarySource;

use super::LibraryWindowUi;

pub(super) fn refresh_album_link_row(ui: &LibraryWindowUi, source: &LibrarySource) {
    let name = match source {
        LibrarySource::Album { name, .. }
        | LibrarySource::AlbumLocal { name, .. }
        | LibrarySource::AlbumUnified { name, .. } => name,
        _ => {
            ui.album_link_row.set_visible(false);
            if let Some(parent) = ui.album_link_row.parent() {
                parent.set_visible(false);
            }
            return;
        }
    };

    ui.album_link_row.set_visible(true);
    if let Some(parent) = ui.album_link_row.parent() {
        parent.set_visible(true);
    }

    let entries = ui.ctx.live_watch_paths.lock().clone();
    match crate::config::watch_entry_for_album(name, &entries) {
        Some(entry) => {
            ui.album_link_row.set_title("Linked folder");
            ui.album_link_row.set_subtitle(entry.path());
            ui.album_link_button.set_label("Unlink");
            ui.album_sync_button.set_visible(true);
        }
        None => {
            ui.album_link_row.set_title("No local folder linked");
            ui.album_link_row
                .set_subtitle("Drop files in the linked folder to sync this album");
            ui.album_link_button.set_label("Link folder\u{2026}");
            ui.album_sync_button.set_visible(false);
        }
    }
}

pub(super) fn connect_album_link_row(ui: Rc<LibraryWindowUi>, _listbox: gtk::ListBox) {
    ui.album_link_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| handle_album_link_click(ui.clone())
    ));
    ui.album_sync_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| handle_album_sync_click(ui.clone())
    ));
}

fn handle_album_sync_click(ui: Rc<LibraryWindowUi>) {
    let source = ui.ctx.library_state.lock().source.clone();
    let LibrarySource::Album {
        id: album_id,
        name: album_name,
    } = source
    else {
        return;
    };
    let entries = ui.ctx.live_watch_paths.lock().clone();
    let Some(entry) = crate::config::watch_entry_for_album(&album_name, &entries) else {
        return;
    };
    let watch_path = std::path::PathBuf::from(entry.path());
    // Sync from the album view should preview every operation regardless of
    // per-folder gates — the user gets explicit checkboxes for each direction
    // in the confirmation dialog and can opt in even when the folder's rules
    // would otherwise suppress them.
    let rules = crate::config::FolderRules {
        delete_folder_to_album: true,
        delete_album_to_folder: true,
        sync_method: crate::config::FolderSyncMethod::Full,
        ..entry.rules()
    };

    let ui_for_async = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        let diff = match crate::library::album_sync::diff_album_vs_folder(
            ui_for_async.ctx.clone(),
            &album_id,
            &watch_path,
            &rules,
            true,
        )
        .await
        {
            Ok(d) => d,
            Err(err) => {
                log::error!("Album diff failed: {}", err);
                return;
            }
        };
        present_sync_dialog(ui_for_async, album_id, album_name, watch_path, diff);
    });
}

fn present_sync_dialog(
    ui: Rc<LibraryWindowUi>,
    album_id: String,
    album_name: String,
    watch_path: std::path::PathBuf,
    diff: crate::library::album_sync::AlbumDiff,
) {
    let upload_count = diff.to_upload.len();
    let download_count = diff.to_download.len();
    let remote_delete_count = diff.to_delete_remote.len();
    let local_delete_count = diff.to_delete_local.len();

    if upload_count == 0
        && download_count == 0
        && remote_delete_count == 0
        && local_delete_count == 0
    {
        let msg = if diff.remote_unhashed > 0 {
            format!(
                "Already in sync. ({} remote item(s) couldn't be matched — missing checksum.)",
                diff.remote_unhashed
            )
        } else {
            "Already in sync.".to_string()
        };
        let info = libadwaita::AlertDialog::builder()
            .heading("Album sync")
            .body(msg)
            .build();
        info.add_response("ok", "OK");
        info.set_default_response(Some("ok"));
        info.set_close_response("ok");
        info.present(Some(&ui.window));
        return;
    }

    let dialog = libadwaita::AlertDialog::builder()
        .heading("Sync album")
        .body(format!(
            "Pick which directions to apply.{}",
            if diff.remote_unhashed > 0 {
                format!(
                    "\n\n{} remote item(s) couldn't be matched (missing checksum).",
                    diff.remote_unhashed
                )
            } else {
                String::new()
            }
        ))
        .build();

    let body_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    let upload_check = gtk::CheckButton::builder()
        .label(format!("Upload {} item(s) to album", upload_count))
        .active(upload_count > 0)
        .sensitive(upload_count > 0)
        .build();
    let download_check = gtk::CheckButton::builder()
        .label(format!("Download {} item(s) to folder", download_count))
        .active(download_count > 0)
        .sensitive(download_count > 0)
        .build();
    let remote_delete_check = gtk::CheckButton::builder()
        .label(format!(
            "Move {} album item(s) to trash",
            remote_delete_count
        ))
        .active(remote_delete_count > 0)
        .sensitive(remote_delete_count > 0)
        .build();
    let local_delete_check = gtk::CheckButton::builder()
        .label(format!(
            "Move {} local item(s) to trash",
            local_delete_count
        ))
        .active(local_delete_count > 0)
        .sensitive(local_delete_count > 0)
        .build();
    if upload_count > 0 {
        body_box.append(&upload_check);
    }
    if download_count > 0 {
        body_box.append(&download_check);
    }
    if remote_delete_count > 0 {
        body_box.append(&remote_delete_check);
    }
    if local_delete_count > 0 {
        body_box.append(&local_delete_check);
    }
    dialog.set_extra_child(Some(&body_box));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("apply", "Apply");
    dialog.set_response_appearance("apply", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("apply"));
    dialog.set_close_response("cancel");

    let ui_for_apply = ui.clone();
    dialog.connect_response(None, move |dlg, response| {
        if response != "apply" {
            return;
        }
        let do_upload = upload_check.is_active();
        let do_download = download_check.is_active();
        let do_remote_delete = remote_delete_check.is_active();
        let do_local_delete = local_delete_check.is_active();
        if !do_upload && !do_download && !do_remote_delete && !do_local_delete {
            dlg.close();
            return;
        }
        let ui = ui_for_apply.clone();
        let album_id = album_id.clone();
        let album_name = album_name.clone();
        let watch_path = watch_path.clone();
        let filtered = crate::library::album_sync::AlbumDiff {
            to_upload: if do_upload {
                diff.to_upload.clone()
            } else {
                Vec::new()
            },
            to_download: if do_download {
                diff.to_download.clone()
            } else {
                Vec::new()
            },
            to_delete_remote: if do_remote_delete {
                diff.to_delete_remote.clone()
            } else {
                Vec::new()
            },
            to_delete_local: if do_local_delete {
                diff.to_delete_local.clone()
            } else {
                Vec::new()
            },
            remote_unhashed: 0,
        };
        glib::MainContext::default().spawn_local(execute_sync_selections(
            ui, album_id, album_name, watch_path, filtered,
        ));
        dlg.close();
    });
    dialog.present(Some(&ui.window));
}

async fn execute_sync_selections(
    ui: Rc<LibraryWindowUi>,
    album_id: String,
    album_name: String,
    watch_path: std::path::PathBuf,
    diff: crate::library::album_sync::AlbumDiff,
) {
    let queued =
        execute_selected_uploads(&ui, &album_id, &album_name, &watch_path, diff.to_upload).await;
    let (downloaded, failed) =
        execute_selected_downloads(&ui, &album_id, &album_name, watch_path, diff.to_download).await;
    let remote_deleted =
        execute_selected_remote_deletes(&ui, &album_id, diff.to_delete_remote).await;
    let (local_deleted, local_delete_failed) =
        execute_selected_local_deletes(&ui, diff.to_delete_local).await;

    log::info!(
        "Album sync done: {} queued for upload, {} downloaded, {} download failures, {} moved to Immich trash, {} local trashed, {} local trash failures",
        queued,
        downloaded,
        failed,
        remote_deleted,
        local_deleted,
        local_delete_failed
    );
    if queued > 0 || downloaded > 0 || remote_deleted > 0 || local_deleted > 0 {
        super::refresh_library_after_mutation(ui.clone(), true);
    }
}

async fn execute_selected_uploads(
    ui: &Rc<LibraryWindowUi>,
    album_id: &str,
    album_name: &str,
    watch_path: &std::path::Path,
    assets: Vec<crate::library::album_sync::LocalEntry>,
) -> usize {
    if assets.is_empty() {
        return 0;
    }
    crate::library::album_sync::execute_uploads(
        ui.ctx.clone(),
        album_id.to_string(),
        album_name.to_string(),
        watch_path.to_path_buf(),
        assets,
    )
    .await
}

async fn execute_selected_downloads(
    ui: &Rc<LibraryWindowUi>,
    album_id: &str,
    album_name: &str,
    watch_path: std::path::PathBuf,
    assets: Vec<crate::api_client::LibraryAsset>,
) -> (usize, usize) {
    if assets.is_empty() {
        return (0, 0);
    }
    crate::library::album_sync::execute_downloads(
        ui.ctx.clone(),
        watch_path,
        Some(album_id.to_string()),
        Some(album_name.to_string()),
        assets,
    )
    .await
}

async fn execute_selected_remote_deletes(
    ui: &Rc<LibraryWindowUi>,
    album_id: &str,
    assets: Vec<crate::api_client::LibraryAsset>,
) -> usize {
    if assets.is_empty() {
        return 0;
    }
    crate::library::album_sync::execute_remote_deletions(ui.ctx.clone(), album_id, assets).await
}

async fn execute_selected_local_deletes(
    ui: &Rc<LibraryWindowUi>,
    assets: Vec<crate::library::album_sync::LocalEntry>,
) -> (usize, usize) {
    if assets.is_empty() {
        return (0, 0);
    }
    crate::library::album_sync::execute_local_deletions(ui.ctx.clone(), assets).await
}

fn handle_album_link_click(ui: Rc<LibraryWindowUi>) {
    let source = ui.ctx.library_state.lock().source.clone();
    let LibrarySource::Album {
        id: album_id,
        name: album_name,
    } = source
    else {
        return;
    };

    let entries = ui.ctx.live_watch_paths.lock().clone();
    let already_linked = crate::config::watch_entry_for_album(&album_name, &entries).is_some();

    if already_linked {
        unlink_album(ui.clone(), &album_name);
        return;
    }

    let dialog = gtk::FileDialog::builder()
        .title(format!("Link folder for album '{}'", album_name))
        .build();
    let ui_for_pick = ui.clone();
    let album_name_for_pick = album_name.clone();
    let album_id_for_pick = album_id.clone();
    dialog.select_folder(Some(&ui.window), gtk::gio::Cancellable::NONE, move |res| {
        let Ok(folder) = res else { return };
        let Some(path) = folder.path() else { return };
        link_album_to_path(
            ui_for_pick.clone(),
            album_id_for_pick.clone(),
            album_name_for_pick.clone(),
            path,
        );
    });
}

fn unlink_album(ui: Rc<LibraryWindowUi>, album_name: &str) {
    {
        let mut config = ui.ctx.config.write();
        config
            .data
            .watch_paths
            .retain(|entry| entry.album_name() != Some(album_name));
        if !config.save() {
            log::error!("Failed to save config after unlink");
            return;
        }
        *ui.ctx.live_watch_paths.lock() = config.data.watch_paths.clone();
    }
    let source_after = ui.ctx.library_state.lock().source.clone();
    refresh_album_link_row(&ui, &source_after);
}

fn link_album_to_path(
    ui: Rc<LibraryWindowUi>,
    album_id: String,
    album_name: String,
    path: std::path::PathBuf,
) {
    let path_string = path.to_string_lossy().to_string();
    {
        let mut config = ui.ctx.config.write();
        config
            .data
            .watch_paths
            .retain(|entry| entry.album_name() != Some(album_name.as_str()));
        config
            .data
            .watch_paths
            .push(crate::config::WatchPathEntry::WithConfig {
                path: path_string,
                album_id: Some(album_id),
                album_name: Some(album_name),
                rules: crate::config::FolderRules::default(),
            });
        if !config.save() {
            log::error!("Failed to save config after link");
            return;
        }
        *ui.ctx.live_watch_paths.lock() = config.data.watch_paths.clone();
    }
    let source_after = ui.ctx.library_state.lock().source.clone();
    refresh_album_link_row(&ui, &source_after);
}
