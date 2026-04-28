//! Library view module -- browse, search, and download assets from an Immich server.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{LibraryAsset, ThumbnailSize};
use crate::app_context::AppContext;
use crate::config::Config;
use crate::library::asset_object::AssetObject;
use crate::library::grid_view::{GridViewParts, build_grid_view, replace_model};
use crate::library::sidebar::{SidebarParts, build_sidebar};
use crate::library::state::{LibraryLoadState, LibrarySortMode, LibrarySource};
use crate::settings_window::build_settings_window;

pub mod asset_object;
pub mod grid_view;
pub mod sidebar;
pub mod state;
pub mod thumbnail_cache;

const PAGE_SIZE: u32 = 50;

struct LibraryWindowUi {
    ctx: Arc<AppContext>,
    app: libadwaita::Application,
    window: libadwaita::ApplicationWindow,
    sidebar: SidebarParts,
    grid: GridViewParts,
    content_stack: gtk::Stack,
    error_label: gtk::Label,
    footer_label: gtk::Label,
    route_label: gtk::Label,
    search_entry: gtk::SearchEntry,
    search_mode: gtk::DropDown,
    sort_mode: gtk::DropDown,
}

pub fn build_library_window(app: &libadwaita::Application, ctx: Arc<AppContext>) {
    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Mimick Library")
        .default_width(1180)
        .default_height(780)
        .build();

    let header = libadwaita::HeaderBar::builder().build();
    let prefs_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Open Settings")
        .build();
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .build();
    header.pack_end(&prefs_button);
    header.pack_end(&refresh_button);

    let toolbar = libadwaita::ToolbarView::builder().build();
    toolbar.add_top_bar(&header);

    let sidebar = build_sidebar();
    let grid = build_grid_view(ctx.clone());

    let search_mode_model = gtk::StringList::new(&["Metadata", "Smart"]);
    let search_mode = gtk::DropDown::builder()
        .model(&search_mode_model)
        .selected(0)
        .build();
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search the library")
        .hexpand(true)
        .build();
    let sort_model = gtk::StringList::new(&["Newest", "Filename", "File Type", "Sync State"]);
    let sort_mode = gtk::DropDown::builder()
        .model(&sort_model)
        .selected(0)
        .build();

    let controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    controls.append(&search_mode);
    controls.append(&search_entry);
    controls.append(&sort_mode);

    let content_stack = gtk::Stack::builder().vexpand(true).hexpand(true).build();
    let loading_label = gtk::Label::builder()
        .label("Loading library assets...")
        .vexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let empty_label = gtk::Label::builder()
        .label("This view is empty")
        .vexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let error_label = gtk::Label::builder()
        .label("Library data could not be loaded")
        .wrap(true)
        .vexpand(true)
        .valign(gtk::Align::Center)
        .build();
    content_stack.add_named(&loading_label, Some("loading"));
    content_stack.add_named(&empty_label, Some("empty"));
    content_stack.add_named(&error_label, Some("error"));
    content_stack.add_named(&grid.scrolled, Some("grid"));

    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let route_label = gtk::Label::builder().xalign(0.0).build();
    let footer_label = gtk::Label::builder().xalign(0.0).wrap(true).build();
    footer.append(&route_label);
    footer.append(&footer_label);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&controls);
    content.append(&content_stack);
    content.append(&footer);

    let split = libadwaita::OverlaySplitView::builder()
        .sidebar(&sidebar.root)
        .content(&content)
        .show_sidebar(true)
        .enable_show_gesture(true)
        .build();
    toolbar.set_content(Some(&split));
    window.set_content(Some(&toolbar));

    let ui = Rc::new(LibraryWindowUi {
        ctx,
        app: app.clone(),
        window: window.clone(),
        sidebar,
        grid,
        content_stack,
        error_label,
        footer_label,
        route_label,
        search_entry,
        search_mode,
        sort_mode,
    });

    connect_sidebar_handlers(ui.clone());
    connect_controls(ui.clone(), prefs_button, refresh_button);
    connect_grid_handlers(ui.clone());

    bootstrap_window(ui);
    window.present();
}

fn bootstrap_window(ui: Rc<LibraryWindowUi>) {
    let initial_request = {
        let mut state = ui.ctx.library_state.lock().unwrap();
        state.load_initial_source()
    };

    load_albums(ui.clone());
    load_status(ui.clone());
    load_source_page(ui, initial_request, false);
}

fn connect_controls(
    ui: Rc<LibraryWindowUi>,
    prefs_button: gtk::Button,
    refresh_button: gtk::Button,
) {
    prefs_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            build_settings_window(&ui.app, ui.ctx.clone());
        }
    ));

    refresh_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            load_albums(ui.clone());
            load_status(ui.clone());
            let request = {
                let source = ui.ctx.library_state.lock().unwrap().source.clone();
                ui.ctx.library_state.lock().unwrap().switch_source(source)
            };
            load_source_page(ui.clone(), request, false);
        }
    ));

    ui.search_entry.connect_activate(clone!(
        #[strong]
        ui,
        move |entry| {
            let query = entry.text().trim().to_string();
            if query.is_empty() {
                return;
            }

            let source = if ui.search_mode.selected() == 1 {
                LibrarySource::SmartSearch { query }
            } else {
                LibrarySource::MetadataSearch { query }
            };
            let request = ui.ctx.library_state.lock().unwrap().switch_source(source);
            load_source_page(ui.clone(), request, false);
        }
    ));

    ui.search_entry.connect_search_changed(clone!(
        #[strong]
        ui,
        move |entry| {
            if !entry.text().trim().is_empty() {
                return;
            }

            let request = ui
                .ctx
                .library_state
                .lock()
                .unwrap()
                .clear_search_restore_previous_source();
            if let Some(request) = request {
                load_source_page(ui.clone(), request, false);
            }
        }
    ));

    ui.sort_mode.connect_selected_notify(clone!(
        #[strong]
        ui,
        move |dropdown| {
            let sort_mode = match dropdown.selected() {
                1 => LibrarySortMode::Filename,
                2 => LibrarySortMode::FileType,
                3 => LibrarySortMode::SyncState,
                _ => LibrarySortMode::NewestFirst,
            };

            let objects = {
                let mut state = ui.ctx.library_state.lock().unwrap();
                state.apply_sort(sort_mode);
                asset_objects_from_state(&state.assets, &ui.ctx)
            };
            replace_model(&ui.grid.model, &objects);
        }
    ));

    ui.sidebar.refresh_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            load_albums(ui.clone());
            let request = {
                let source = ui.ctx.library_state.lock().unwrap().source.clone();
                ui.ctx.library_state.lock().unwrap().switch_source(source)
            };
            load_source_page(ui.clone(), request, false);
        }
    ));

    ui.sidebar.delete_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            let current = ui.ctx.library_state.lock().unwrap().source.clone();
            let LibrarySource::Album { id, .. } = current else {
                return;
            };

            let dialog = libadwaita::AlertDialog::builder()
                .heading("Delete Album?")
                .body(
                    "This removes the album container from Immich but leaves the assets untouched.",
                )
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("delete", "Delete");
            dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
            dialog.connect_response(
                None,
                clone!(
                    #[strong]
                    ui,
                    #[strong]
                    id,
                    move |dialog, response| {
                        dialog.close();
                        if response != "delete" {
                            return;
                        }
                        glib::MainContext::default().spawn_local(clone!(
                            #[strong]
                            ui,
                            #[strong]
                            id,
                            async move {
                                if let Err(err) = ui.ctx.api_client.delete_album(&id).await {
                                    ui.error_label
                                        .set_label(&format!("Album delete failed: {}", err));
                                    ui.content_stack.set_visible_child_name("error");
                                    return;
                                }

                                let request = {
                                    let mut state = ui.ctx.library_state.lock().unwrap();
                                    state.albums.retain(|album| album.id != id);
                                    state.switch_source(LibrarySource::AllAssets)
                                };
                                reload_sidebar(&ui);
                                load_source_page(ui.clone(), request, false);
                            }
                        ));
                    }
                ),
            );
            dialog.present(Some(&ui.window));
        }
    ));
}

fn connect_sidebar_handlers(ui: Rc<LibraryWindowUi>) {
    ui.sidebar.list.connect_row_selected(clone!(
        #[strong]
        ui,
        move |_, row| {
            let Some(row) = row else {
                return;
            };

            let source = row
                .tooltip_text()
                .map(|tooltip| {
                    if tooltip == "all-assets" {
                        LibrarySource::AllAssets
                    } else {
                        let mut parts = tooltip.splitn(2, ':');
                        let id = parts.next().unwrap_or_default().to_string();
                        let name = parts.next().unwrap_or("Album").to_string();
                        LibrarySource::Album { id, name }
                    }
                })
                .unwrap_or(LibrarySource::AllAssets);

            let request = ui.ctx.library_state.lock().unwrap().switch_source(source);
            load_source_page(ui.clone(), request, false);
        }
    ));
}

fn connect_grid_handlers(ui: Rc<LibraryWindowUi>) {
    ui.grid.view.connect_activate(clone!(
        #[strong]
        ui,
        move |_, position| {
            let Some(item) = ui.grid.model.item(position).and_downcast::<AssetObject>() else {
                return;
            };
            let asset_id = item.property::<String>("id");
            let filename = item.property::<String>("filename");

            open_lightbox(ui.clone(), asset_id, filename);
        }
    ));

    ui.grid.scrolled.vadjustment().connect_value_changed(clone!(
        #[strong]
        ui,
        move |adj| {
            let threshold = (adj.upper() - adj.page_size()) * 0.75;
            if adj.value() < threshold {
                return;
            }

            let next = ui
                .ctx
                .library_state
                .lock()
                .unwrap()
                .load_next_page_if_needed();
            if let Some(request) = next {
                load_source_page(ui.clone(), request, true);
            }
        }
    ));
}

fn load_albums(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            match ui.ctx.api_client.fetch_library_albums().await {
                Ok(albums) => {
                    ui.ctx.library_state.lock().unwrap().load_albums(albums);
                    reload_sidebar(&ui);
                }
                Err(err) => {
                    ui.error_label
                        .set_label(&format!("Could not load albums: {}", err));
                    ui.content_stack.set_visible_child_name("error");
                }
            }
        }
    ));
}

fn load_status(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let stats = ui.ctx.api_client.fetch_server_stats().await.ok();
            let about = ui.ctx.api_client.fetch_server_about().await.ok();
            let route = ui.ctx.api_client.active_route_label().await;

            {
                let mut state = ui.ctx.library_state.lock().unwrap();
                state.set_status(stats, about);
            }
            update_footer(&ui, route);
        }
    ));
}

fn load_source_page(ui: Rc<LibraryWindowUi>, request: (u64, LibrarySource, u32), append: bool) {
    ui.content_stack.set_visible_child_name("loading");
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let (generation, source, page) = request;
            let result = match source.clone() {
                LibrarySource::AllAssets => {
                    ui.ctx.api_client.search_metadata("", page, PAGE_SIZE).await
                }
                LibrarySource::Album { id, .. } => {
                    ui.ctx
                        .api_client
                        .fetch_album_assets(&id, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::SmartSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_smart(&query, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::MetadataSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE)
                        .await
                }
            };

            match result {
                Ok(items) => {
                    let objects = {
                        let mut state = ui.ctx.library_state.lock().unwrap();
                        let applied = if append {
                            state.append_assets(generation, items)
                        } else {
                            state.replace_assets(generation, items)
                        };
                        if !applied {
                            return;
                        }
                        asset_objects_from_state(&state.assets, &ui.ctx)
                    };
                    replace_model(&ui.grid.model, &objects);
                    sync_content_state(&ui);
                    reload_sidebar(&ui);
                }
                Err(err) => {
                    let mut state = ui.ctx.library_state.lock().unwrap();
                    state.mark_error(generation, err.clone());
                    ui.error_label
                        .set_label(&format!("Could not load library assets: {}", err));
                    ui.content_stack.set_visible_child_name("error");
                }
            }
        }
    ));
}

fn reload_sidebar(ui: &LibraryWindowUi) {
    while let Some(row) = ui.sidebar.list.first_child() {
        ui.sidebar.list.remove(&row);
    }

    let selected_source = ui.ctx.library_state.lock().unwrap().source.clone();
    let all_assets = gtk::ListBoxRow::builder()
        .tooltip_text("all-assets")
        .child(
            &libadwaita::ActionRow::builder()
                .title("All Assets")
                .subtitle("Browse the full library")
                .build(),
        )
        .build();
    ui.sidebar.list.append(&all_assets);

    let albums = ui.ctx.library_state.lock().unwrap().albums.clone();
    for album in albums {
        let subtitle = format!("{} asset(s)", album.asset_count);
        let row = gtk::ListBoxRow::builder()
            .tooltip_text(format!("{}:{}", album.id, album.album_name))
            .child(
                &libadwaita::ActionRow::builder()
                    .title(&album.album_name)
                    .subtitle(&subtitle)
                    .build(),
            )
            .build();
        ui.sidebar.list.append(&row);
    }

    match selected_source {
        LibrarySource::AllAssets => {
            ui.sidebar.list.select_row(Some(&all_assets));
            ui.sidebar.delete_button.set_sensitive(false);
        }
        LibrarySource::Album { id, .. } => {
            let mut child = ui.sidebar.list.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
                    && row
                        .tooltip_text()
                        .as_deref()
                        .is_some_and(|tooltip| tooltip.starts_with(&id))
                {
                    ui.sidebar.list.select_row(Some(&row));
                    ui.sidebar.delete_button.set_sensitive(true);
                    break;
                }
                child = next;
            }
        }
        _ => ui.sidebar.delete_button.set_sensitive(false),
    }
}

fn sync_content_state(ui: &LibraryWindowUi) {
    match &ui.ctx.library_state.lock().unwrap().load_state {
        LibraryLoadState::Idle | LibraryLoadState::Loading => {
            ui.content_stack.set_visible_child_name("loading");
        }
        LibraryLoadState::Loaded => {
            ui.content_stack.set_visible_child_name("grid");
        }
        LibraryLoadState::Empty => {
            ui.content_stack.set_visible_child_name("empty");
        }
        LibraryLoadState::Error(message) => {
            ui.error_label.set_label(message);
            ui.content_stack.set_visible_child_name("error");
        }
    }
}

fn update_footer(ui: &LibraryWindowUi, route: Option<String>) {
    let state = ui.ctx.library_state.lock().unwrap();
    if let Some(route) = route {
        ui.route_label.set_label(&format!("Connected ({})", route));
    } else {
        ui.route_label.set_label("Offline");
    }

    let stats = state
        .status
        .stats
        .as_ref()
        .map(|stats| format!("{} photos, {} videos", stats.images, stats.videos))
        .unwrap_or_else(|| "Statistics unavailable".to_string());
    let about = state
        .status
        .about
        .as_ref()
        .map(|about| format!("Immich {}", about.version))
        .unwrap_or_else(|| "Version unavailable".to_string());
    ui.footer_label.set_label(&format!("{} | {}", stats, about));
}

fn asset_objects_from_state(assets: &[LibraryAsset], ctx: &AppContext) -> Vec<AssetObject> {
    let sync_index = ctx.sync_index.lock().unwrap();
    assets
        .iter()
        .map(|asset| {
            let sync_state = asset
                .checksum
                .as_deref()
                .and_then(|checksum| sync_index.local_path_for_checksum(checksum))
                .map(|_| 2)
                .unwrap_or(0);
            AssetObject::new(
                &asset.id,
                &asset.filename,
                &asset.mime_type,
                &asset.created_at,
                &asset.asset_type,
                sync_state,
                asset.thumbhash.as_deref(),
            )
        })
        .collect()
}

fn open_lightbox(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String) {
    let dialog = libadwaita::Window::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title(&filename)
        .default_width(980)
        .default_height(760)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .vexpand(true)
        .hexpand(true)
        .build();
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let download = gtk::Button::builder().label("Download").build();
    let close = gtk::Button::builder().label("Close").build();
    actions.append(&download);
    actions.append(&close);
    content.append(&picture);
    content.append(&actions);
    dialog.set_content(Some(&content));

    close.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| dialog.close()
    ));

    download.connect_clicked(clone!(
        #[strong]
        ui,
        #[strong]
        asset_id,
        #[strong]
        filename,
        move |_| {
            start_download(ui.clone(), asset_id.clone(), filename.clone());
        }
    ));

    glib::MainContext::default().spawn_local(clone!(
        #[weak]
        dialog,
        async move {
            if let Ok(texture) = ui
                .ctx
                .thumbnail_cache
                .load_thumbnail(&asset_id, ThumbnailSize::Preview)
                .await
                && dialog.is_visible()
            {
                picture.set_paintable(Some(&texture));
            }
        }
    ));

    dialog.present();
}

fn start_download(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let Some(target_dir) = ensure_download_target(&ui).await else {
                return;
            };
            let output_path = target_dir.join(&filename);
            if output_path.exists() {
                let dialog = libadwaita::AlertDialog::builder()
                    .heading("File already exists")
                    .body("Overwrite the existing file or skip this download?")
                    .build();
                dialog.add_response("skip", "Skip");
                dialog.add_response("overwrite", "Overwrite");
                dialog.set_response_appearance(
                    "overwrite",
                    libadwaita::ResponseAppearance::Destructive,
                );
                dialog.connect_response(
                    None,
                    clone!(
                        #[strong]
                        ui,
                        #[strong]
                        asset_id,
                        #[strong]
                        filename,
                        move |dialog, response| {
                            dialog.close();
                            if response == "overwrite" {
                                spawn_download(
                                    ui.clone(),
                                    asset_id.clone(),
                                    target_dir.join(&filename),
                                );
                            }
                        }
                    ),
                );
                dialog.present(Some(&ui.window));
                return;
            }
            spawn_download(ui, asset_id, output_path);
        }
    ));
}

fn spawn_download(ui: Rc<LibraryWindowUi>, asset_id: String, output_path: PathBuf) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            match ui.ctx.api_client.download_original(&asset_id).await {
                Ok(bytes) => {
                    let path_for_write = output_path.clone();
                    let write_result =
                        tokio::task::spawn_blocking(move || std::fs::write(&path_for_write, bytes))
                            .await;
                    let (heading, body) = match write_result {
                        Ok(Ok(())) => (
                            "Download Complete",
                            format!("Saved {}", output_path.display()),
                        ),
                        Ok(Err(err)) => {
                            ("Download Failed", format!("Could not write file: {}", err))
                        }
                        Err(err) => (
                            "Download Failed",
                            format!("The download task could not complete: {}", err),
                        ),
                    };
                    let alert = libadwaita::AlertDialog::builder()
                        .heading(heading)
                        .body(&body)
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

async fn ensure_download_target(ui: &LibraryWindowUi) -> Option<PathBuf> {
    let config = Config::new();
    if let Some(path) = config.data.download_target_path {
        return Some(PathBuf::from(path));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = gtk::FileDialog::builder()
        .title("Choose Library Download Folder")
        .build();
    dialog.select_folder(Some(&ui.window), gtk::gio::Cancellable::NONE, move |res| {
        let _ = tx.send(
            res.ok()
                .and_then(|folder| folder.path())
                .map(|path| path.to_path_buf()),
        );
    });

    let path = rx.await.ok().flatten()?;
    let mut config = Config::new();
    config.data.download_target_path = Some(path.to_string_lossy().to_string());
    let _ = config.save();
    Some(path)
}
