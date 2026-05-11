//! Library view module -- browse, search, and download assets from an Immich server.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{
    LibraryAsset, MetadataSearchFilters, ThumbnailSize, TransferProgressCallback,
};
use crate::app_context::AppContext;
use crate::library::albums_view::{
    AlbumClick, AlbumsViewParts, build_albums_view, populate_albums,
};
use crate::library::asset_object::AssetObject;
use crate::library::explore_view::{ExploreViewParts, build_explore_view};
use crate::library::grid_view::{GridViewParts, build_grid_view};
use crate::library::local_source::{
    LocalAsset, enumerate_local, enumerate_local_for_entry, filter_by_filename,
};
use crate::library::sidebar::{SidebarParts, build_sidebar};
use crate::library::state::{LibraryLoadState, LibrarySortMode, LibrarySource};
use crate::settings_window::{build_settings_window_with_parent, show_queue_inspector};
use crate::state_manager::TransferDirection;

pub(super) const LOCAL_ID_PREFIX: &str = "local::";

pub mod album_sync;
pub mod albums_view;
pub mod asset_model;
pub mod asset_object;
pub mod explore_view;
pub mod grid_view;
pub mod local_source;
pub mod sidebar;
pub mod state;
pub mod style;
pub mod thumbnail_cache;

const PAGE_SIZE: u32 = 50;

fn begin_download_session(ctx: &Arc<AppContext>, item_label: String) {
    let state_ref = ctx.state.clone();
    let mut state = state_ref.lock();
    let route = state.active_server_route.clone();
    state
        .transfer
        .begin_group(TransferDirection::Download, Some(item_label), route);
}

fn track_download_item(
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

fn finish_download_item(ctx: &Arc<AppContext>, item_id: &str) {
    let mut state = ctx.state.lock();
    let route = state.active_server_route.clone();
    state
        .transfer
        .finish_item(TransferDirection::Download, item_id, route);
}

fn register_app_icons() {
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        theme.add_resource_path("/dev/nicx/mimick/icons");
    }
}

fn load_texture_oriented(path: &std::path::Path) -> Option<gdk4::Texture> {
    let raw = gtk::gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let pixbuf = raw.apply_embedded_orientation().unwrap_or(raw);
    #[allow(deprecated)]
    Some(gdk4::Texture::for_pixbuf(&pixbuf))
}

struct LibraryWindowUi {
    ctx: Arc<AppContext>,
    app: libadwaita::Application,
    window: libadwaita::ApplicationWindow,
    nav: libadwaita::NavigationView,
    sidebar: SidebarParts,
    grid: GridViewParts,
    explore: ExploreViewParts,
    albums: AlbumsViewParts,
    content_stack: gtk::Stack,
    error_label: gtk::Label,
    transfer_bar: gtk::Box,
    transfer_progress: gtk::ProgressBar,
    transfer_icon: gtk::Image,
    transfer_label: gtk::Label,
    search_entry: gtk::SearchEntry,
    search_mode: gtk::DropDown,
    sort_mode: gtk::DropDown,
    source_mode: gtk::DropDown,
    filters_button: gtk::Button,
    timeline_toggle: gtk::ToggleButton,
    timeline_banner: gtk::Label,
    source_mode_suppressed: Cell<bool>,
    sidebar_suppressed: Cell<bool>,
    back_button: gtk::Button,
    select_toggle: gtk::ToggleButton,
    bulk_bar: gtk::Revealer,
    bulk_count_label: gtk::Label,
    album_link_row: libadwaita::ActionRow,
    album_link_button: gtk::Button,
    album_sync_button: gtk::Button,
    last_seen_upload_batch: Cell<u64>,
}

pub fn build_library_window(app: &libadwaita::Application, ctx: Arc<AppContext>) {
    style::ensure_registered();
    register_app_icons();

    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Mimick Library")
        .name("mimick-library-window")
        .default_width(1180)
        .default_height(780)
        .build();

    let header = libadwaita::HeaderBar::builder()
        .show_start_title_buttons(true)
        .show_end_title_buttons(true)
        .build();
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle sidebar (F9)")
        .active(true)
        .build();
    let back_button = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Back (Alt+Left)")
        .sensitive(false)
        .build();
    let prefs_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Open Settings")
        .build();
    let queue_button = gtk::Button::builder()
        .icon_name("view-list-symbolic")
        .tooltip_text("Open Queue Inspector")
        .build();
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .build();
    header.pack_start(&sidebar_toggle);
    header.pack_start(&back_button);
    header.pack_end(&prefs_button);
    header.pack_end(&queue_button);
    header.pack_end(&refresh_button);
    let select_toggle = gtk::ToggleButton::builder()
        .icon_name("checkbox-symbolic")
        .tooltip_text("Select assets (Esc to exit)")
        .build();

    let toolbar = libadwaita::ToolbarView::builder().build();
    toolbar.add_top_bar(&header);

    let sidebar = build_sidebar();
    let grid = build_grid_view(ctx.clone(), select_toggle.clone());
    let explore = build_explore_view();
    let albums = build_albums_view();

    let source_mode_model = gtk::StringList::new(&["Remote", "Local", "Unified"]);
    let source_mode = gtk::DropDown::builder()
        .model(&source_mode_model)
        .selected(0)
        .tooltip_text("Asset source")
        .build();
    let timeline_toggle = gtk::ToggleButton::builder()
        .label("Timeline")
        .tooltip_text("Timeline view (all assets only)")
        .build();

    // Three distinct search dimensions, each routed to a different Immich
    // endpoint shape. Smart and OCR are *separate* fields on the Immich
    // search DTOs (`query` vs `ocr` per the live OpenAPI spec), so we
    // expose them independently rather than collapsing OCR into Smart.
    let search_mode_model = gtk::StringList::new(&["Filename", "Smart Search", "OCR"]);
    let search_mode = gtk::DropDown::builder()
        .model(&search_mode_model)
        .selected(0)
        .tooltip_text(
            "Filename: matches the file name and EXIF metadata.\n\
             Smart: CLIP-based semantic search — natural-language queries against visual scenes \
             (\"sunset beach\", \"birthday cake\", \"invoices\").\n\
             OCR: matches text recognised inside images by Immich's ML pipeline. Faster than \
             Smart since it skips CLIP inference.",
        )
        .build();
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search filenames")
        .hexpand(true)
        .build();
    let filters_button = gtk::Button::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("Advanced filters (date, location, camera, EXIF)")
        .build();
    let sort_model = gtk::StringList::new(&["Newest", "Filename", "File Type"]);
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
    controls.append(&source_mode);
    controls.append(&timeline_toggle);
    controls.append(&search_mode);
    controls.append(&search_entry);
    controls.append(&filters_button);
    controls.append(&sort_mode);

    let timeline_banner = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["mimick-timeline-banner".to_string()])
        .visible(false)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(12)
        .build();

    let content_stack = gtk::Stack::builder().vexpand(true).hexpand(true).build();
    let loading_view = build_status_view(
        "view-refresh-symbolic",
        "Loading…",
        "Fetching library data from the Immich server",
    );
    let empty_view = build_status_view(
        "image-x-generic-symbolic",
        "Nothing to show",
        "No assets match the current view",
    );
    let error_view = build_status_view(
        "dialog-warning-symbolic",
        "Library data unavailable",
        "Could not load library assets",
    );
    let error_label = error_view
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("status-view subtitle label");
    content_stack.add_named(&loading_view, Some("loading"));
    content_stack.add_named(&empty_view, Some("empty"));
    content_stack.add_named(&error_view, Some("error"));
    content_stack.add_named(&grid.scrolled, Some("grid"));
    content_stack.add_named(&explore.root, Some("explore"));
    content_stack.add_named(&albums.root, Some("albums"));

    let transfer_progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(vec!["mimick-transfer-progress".to_string()])
        .build();
    let transfer_icon = gtk::Image::builder()
        .icon_size(gtk::IconSize::Normal)
        .css_classes(vec!["dim-label".to_string()])
        .visible(false)
        .build();
    let transfer_label = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .max_width_chars(48)
        .css_classes(vec!["caption".to_string(), "dim-label".to_string()])
        .build();
    let transfer_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .css_classes(vec!["mimick-transfer-shell".to_string()])
        .build();
    transfer_bar.append(&transfer_progress);
    transfer_bar.append(&transfer_icon);
    transfer_bar.append(&transfer_label);

    let album_link_row = libadwaita::ActionRow::builder()
        .title("No local folder linked")
        .subtitle("Drop files in the linked folder to sync this album")
        .build();
    let album_sync_button = gtk::Button::builder()
        .label("Sync…")
        .valign(gtk::Align::Center)
        .css_classes(vec!["suggested-action".to_string()])
        .visible(false)
        .build();
    let album_link_button = gtk::Button::builder()
        .label("Link folder…")
        .valign(gtk::Align::Center)
        .build();
    album_link_row.add_suffix(&album_sync_button);
    album_link_row.add_suffix(&album_link_button);
    let album_link_listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .margin_bottom(4)
        .visible(false)
        .build();
    album_link_listbox.append(&album_link_row);

    let bulk_count_label = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    let bulk_delete = gtk::Button::builder()
        .label("Delete")
        .css_classes(vec!["destructive-action".to_string()])
        .build();
    let bulk_download = gtk::Button::builder().label("Download").build();
    let bulk_clear = gtk::Button::builder()
        .label("Clear")
        .css_classes(vec!["flat".to_string()])
        .build();
    let bulk_inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .css_classes(vec!["toolbar".to_string()])
        .build();
    bulk_inner.append(&bulk_count_label);
    bulk_inner.append(&bulk_clear);
    bulk_inner.append(&bulk_download);
    bulk_inner.append(&bulk_delete);
    let bulk_bar = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(false)
        .child(&bulk_inner)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&controls);
    content.append(&album_link_listbox);
    content.append(&timeline_banner);
    content.append(&content_stack);
    content.append(&bulk_bar);
    content.append(&transfer_bar);

    let split = libadwaita::OverlaySplitView::builder()
        .sidebar(&sidebar.root)
        .content(&content)
        .show_sidebar(true)
        .enable_show_gesture(true)
        .build();
    split
        .bind_property("show-sidebar", &sidebar_toggle, "active")
        .sync_create()
        .bidirectional()
        .build();
    toolbar.set_content(Some(&split));

    let nav = libadwaita::NavigationView::new();
    let root_page = libadwaita::NavigationPage::builder()
        .child(&toolbar)
        .title("Library")
        .can_pop(false)
        .build();
    nav.add(&root_page);
    window.set_content(Some(&nav));

    let f9 = gtk::Shortcut::builder()
        .trigger(&gtk::ShortcutTrigger::parse_string("F9").unwrap())
        .action(&gtk::CallbackAction::new(clone!(
            #[strong]
            split,
            move |_, _| {
                split.set_show_sidebar(!split.shows_sidebar());
                glib::Propagation::Stop
            }
        )))
        .build();
    let shortcut_controller = gtk::ShortcutController::new();
    shortcut_controller.add_shortcut(f9);
    window.add_controller(shortcut_controller);

    let ui = Rc::new(LibraryWindowUi {
        ctx,
        app: app.clone(),
        window: window.clone(),
        nav: nav.clone(),
        sidebar,
        grid,
        explore,
        albums,
        content_stack,
        error_label,
        transfer_bar,
        transfer_progress,
        transfer_icon,
        transfer_label,
        search_entry,
        search_mode,
        sort_mode,
        source_mode,
        filters_button: filters_button.clone(),
        timeline_toggle,
        timeline_banner,
        source_mode_suppressed: Cell::new(false),
        sidebar_suppressed: Cell::new(false),
        back_button: back_button.clone(),
        select_toggle: select_toggle.clone(),
        bulk_bar: bulk_bar.clone(),
        bulk_count_label: bulk_count_label.clone(),
        album_link_row: album_link_row.clone(),
        album_link_button: album_link_button.clone(),
        album_sync_button: album_sync_button.clone(),
        last_seen_upload_batch: Cell::new(0),
    });
    *ui.grid.context_menu_handler.borrow_mut() = Some(Box::new(clone!(
        #[strong]
        ui,
        move |position, x, y| {
            show_asset_context_menu(ui.clone(), &ui.grid.view, position, x, y);
        }
    )));

    connect_album_link_row(ui.clone(), album_link_listbox);

    connect_select_mode(ui.clone(), select_toggle.clone());
    connect_bulk_actions(ui.clone(), bulk_delete, bulk_download, bulk_clear);

    connect_sidebar_handlers(ui.clone());
    connect_controls(ui.clone(), prefs_button, queue_button, refresh_button);
    connect_grid_handlers(ui.clone());
    connect_filters_button(ui.clone(), filters_button);

    bootstrap_window(ui);
    window.present();
}

fn bootstrap_window(ui: Rc<LibraryWindowUi>) {
    let initial_request = {
        let mut state = ui.ctx.library_state.lock();
        state.load_initial_source()
    };

    apply_timeline_ui_state(&ui, &initial_request.1);
    load_albums(ui.clone());
    load_status(ui.clone());
    fetch_current_user(ui.clone());
    connect_albums_create(ui.clone());
    spawn_server_ping_loop(ui.clone());
    spawn_transfer_poll_loop(ui.clone());
    load_source_page(ui, initial_request, false);
}

fn spawn_server_ping_loop(ui: Rc<LibraryWindowUi>) {
    glib::timeout_add_seconds_local(5, move || {
        let ui_for_tick = ui.clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = ui_for_tick.ctx.api_client.check_connection().await;
            let route = ui_for_tick.ctx.api_client.active_route_label().await;
            update_footer(&ui_for_tick, route);
        });
        glib::ControlFlow::Continue
    });
}

fn spawn_transfer_poll_loop(ui: Rc<LibraryWindowUi>) {
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let completed_batches = ui.ctx.state.lock().completed_upload_batches;
        if completed_batches != ui.last_seen_upload_batch.get() {
            ui.last_seen_upload_batch.set(completed_batches);
            refresh_library_after_mutation(ui.clone(), true);
        }
        update_transfer_ui(&ui);
        glib::ControlFlow::Continue
    });
}

fn fetch_current_user(ui: Rc<LibraryWindowUi>) {
    if ui.ctx.current_user_id.lock().is_some() {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_current_user_id().await {
            Ok(id) => {
                *ui.ctx.current_user_id.lock() = Some(id);
            }
            Err(err) => log::warn!("Could not fetch current user id: {}", err),
        }
    });
}

fn connect_albums_create(ui: Rc<LibraryWindowUi>) {
    ui.albums.create_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| prompt_create_album(ui.clone())
    ));
}

fn prompt_create_album(ui: Rc<LibraryWindowUi>) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Create album")
        .body("Choose a name for the new album.")
        .build();
    let entry = gtk::Entry::builder()
        .placeholder_text("Album name")
        .activates_default(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let ui_for_choice = ui.clone();
    let entry_for_choice = entry.clone();
    dialog.connect_response(None, move |dlg, response| {
        if response != "create" {
            return;
        }
        let name = entry_for_choice.text().to_string();
        if name.trim().is_empty() {
            return;
        }
        let ui = ui_for_choice.clone();
        glib::MainContext::default().spawn_local(async move {
            match ui.ctx.api_client.create_album(name.trim()).await {
                Ok(_) => {
                    refresh_library_after_mutation(ui.clone(), false);
                }
                Err(err) => log::error!("Create album failed: {}", err),
            }
        });
        dlg.close();
    });
    dialog.present(Some(&ui.window));
}

fn refresh_albums_view(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_library_albums().await {
            Ok(albums) => {
                let on_click = album_click_handler(ui.clone());
                populate_albums(&ui.albums, ui.ctx.clone(), albums, on_click);
            }
            Err(err) => log::warn!("Albums fetch failed: {}", err),
        }
    });
}

fn album_click_handler(ui: Rc<LibraryWindowUi>) -> AlbumClick {
    Rc::new(move |id: &str, name: String| {
        sidebar_dispatch(
            ui.clone(),
            LibrarySource::Album {
                id: id.to_string(),
                name,
            },
        );
    })
}

fn connect_controls(
    ui: Rc<LibraryWindowUi>,
    prefs_button: gtk::Button,
    queue_button: gtk::Button,
    refresh_button: gtk::Button,
) {
    prefs_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            build_settings_window_with_parent(&ui.app, ui.ctx.clone(), Some(&ui.window));
        }
    ));

    queue_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            show_queue_inspector(&ui.window, ui.ctx.queue_manager.clone());
        }
    ));

    refresh_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            refresh_library_surfaces(ui.clone(), true);
        }
    ));

    ui.back_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            navigate_back(ui.clone());
        }
    ));

    // Alt+Left → back. Attached to the window so it works regardless of focus.
    let alt_left = gtk::Shortcut::builder()
        .trigger(&gtk::ShortcutTrigger::parse_string("<Alt>Left").unwrap())
        .action(&gtk::CallbackAction::new(clone!(
            #[strong]
            ui,
            move |_, _| {
                if navigate_back(ui.clone()) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        )))
        .build();
    let alt_left_controller = gtk::ShortcutController::new();
    alt_left_controller.add_shortcut(alt_left);
    ui.window.add_controller(alt_left_controller);

    let f5_controller = gtk::EventControllerKey::new();
    f5_controller.connect_key_pressed({
        let refresh_button = refresh_button.clone();
        move |_, keyval, _, _| {
            if keyval == gtk::gdk::Key::F5 {
                refresh_button.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    ui.window.add_controller(f5_controller);

    ui.source_mode.connect_selected_notify(clone!(
        #[strong]
        ui,
        move |dropdown| {
            if ui.source_mode_suppressed.get() {
                return;
            }
            let album_ctx = match ui.ctx.library_state.lock().source.clone() {
                LibrarySource::Album { id, name }
                | LibrarySource::AlbumLocal { id, name }
                | LibrarySource::AlbumUnified { id, name } => Some((id, name)),
                _ => None,
            };
            let source = match (dropdown.selected(), album_ctx) {
                (1, Some((id, name))) => LibrarySource::AlbumLocal { id, name },
                (1, None) => LibrarySource::LocalAll,
                (2, Some((id, name))) => LibrarySource::AlbumUnified { id, name },
                (2, None) => LibrarySource::Unified,
                (_, Some((id, name))) => LibrarySource::Album { id, name },
                (_, None) => {
                    if ui.timeline_toggle.is_active() {
                        LibrarySource::Timeline
                    } else {
                        LibrarySource::AllAssets
                    }
                }
            };
            // Searching while switching sources would require thread-safe
            // re-routing of the search field; clear it on source change.
            ui.search_entry.set_text("");
            let request = ui.ctx.library_state.lock().navigate_to(source);
            apply_timeline_ui_state(&ui, &request.1);
            load_source_page(ui.clone(), request, false);
            update_back_button(&ui);
        }
    ));

    ui.timeline_toggle.connect_toggled(clone!(
        #[strong]
        ui,
        move |toggle| {
            if !toggle.is_sensitive() {
                return;
            }
            let current = ui.ctx.library_state.lock().source.clone();
            if !matches!(current, LibrarySource::AllAssets | LibrarySource::Timeline) {
                toggle.set_active(false);
                return;
            }
            if matches!(current, LibrarySource::Timeline) == toggle.is_active() {
                return;
            }
            ui.search_entry.set_text("");
            let next_source = if toggle.is_active() {
                LibrarySource::Timeline
            } else {
                LibrarySource::AllAssets
            };
            let request = ui.ctx.library_state.lock().navigate_to(next_source);
            apply_timeline_ui_state(&ui, &request.1);
            load_source_page(ui.clone(), request, false);
            update_back_button(&ui);
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

            let source = match ui.source_mode.selected() {
                1 => LibrarySource::LocalSearch { query },
                2 => LibrarySource::UnifiedSearch { query },
                _ => match ui.search_mode.selected() {
                    1 => LibrarySource::SmartSearch { query },
                    2 => LibrarySource::OcrSearch { query },
                    _ => LibrarySource::MetadataSearch { query },
                },
            };
            let request = ui.ctx.library_state.lock().navigate_to(source);
            apply_timeline_ui_state(&ui, &request.1);
            load_source_page(ui.clone(), request, false);
            update_back_button(&ui);
        }
    ));

    ui.search_mode.connect_selected_notify(clone!(
        #[strong]
        ui,
        move |dropdown| {
            let placeholder = match dropdown.selected() {
                1 => "Describe what you're looking for…",
                2 => "Find words shown inside images",
                _ => "Search filenames",
            };
            ui.search_entry.set_placeholder_text(Some(placeholder));
        }
    ));

    ui.search_entry.connect_stop_search(clone!(
        #[strong]
        ui,
        move |entry| {
            entry.set_text("");
            let request = ui
                .ctx
                .library_state
                .lock()
                .clear_search_restore_previous_source();
            if let Some(request) = request {
                apply_timeline_ui_state(&ui, &request.1);
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
                _ => LibrarySortMode::NewestFirst,
            };

            let mut state = ui.ctx.library_state.lock();
            state.apply_sort(sort_mode);
            ui.grid
                .model
                .reset(&ui.ctx, &state.assets, &state.sort_mode);
        }
    ));
}

fn refresh_library_surfaces(ui: Rc<LibraryWindowUi>, include_current_source: bool) {
    load_albums(ui.clone());
    load_status(ui.clone());
    ui.explore.populated.set(false);
    if include_current_source {
        let request = {
            let source = ui.ctx.library_state.lock().source.clone();
            ui.ctx.library_state.lock().switch_source(source)
        };
        load_source_page(ui, request, false);
    }
}

fn refresh_library_after_mutation(ui: Rc<LibraryWindowUi>, prefer_current_source: bool) {
    refresh_library_surfaces(ui, prefer_current_source);
}

fn connect_select_mode(ui: Rc<LibraryWindowUi>, select_toggle: gtk::ToggleButton) {
    let selection = ui.grid.selection.clone();
    let bulk_bar = ui.bulk_bar.clone();
    let count_label = ui.bulk_count_label.clone();

    let refresh = {
        let selection = selection.clone();
        let bulk_bar = bulk_bar.clone();
        let count_label = count_label.clone();
        let select_toggle = select_toggle.clone();
        Rc::new(move || {
            let n = selection_count(&selection);
            bulk_bar.set_reveal_child(select_toggle.is_active() && n > 0);
            count_label.set_label(&format!("{} selected", n));
        })
    };

    selection.connect_selection_changed({
        let refresh = refresh.clone();
        move |_, _, _| (*refresh)()
    });

    select_toggle.connect_toggled({
        let selection = selection.clone();
        let refresh = refresh.clone();
        move |toggle| {
            if !toggle.is_active() {
                selection.unselect_all();
            }
            (*refresh)();
        }
    });

    // Ctrl-hold transient checkboxes: pressing Ctrl reveals the selection UI;
    // releasing Ctrl without having selected anything dismisses it again.
    // Ctrl+click then commits a selection (handled separately in grid_view)
    // and the release no longer collapses select mode.
    let transient = Rc::new(Cell::new(false));
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    key_controller.connect_key_pressed({
        let select_toggle = select_toggle.clone();
        let transient = transient.clone();
        move |_, keyval, _, _| {
            if keyval == gtk::gdk::Key::Escape && select_toggle.is_active() {
                select_toggle.set_active(false);
                transient.set(false);
                return glib::Propagation::Stop;
            }
            if matches!(keyval, gtk::gdk::Key::Control_L | gtk::gdk::Key::Control_R)
                && !select_toggle.is_active()
            {
                select_toggle.set_active(true);
                transient.set(true);
            }
            glib::Propagation::Proceed
        }
    });
    key_controller.connect_key_released({
        let select_toggle = select_toggle.clone();
        let selection = selection.clone();
        let transient = transient.clone();
        move |_, keyval, _, _| {
            if !matches!(keyval, gtk::gdk::Key::Control_L | gtk::gdk::Key::Control_R)
                || !transient.get()
            {
                return;
            }
            if selection.selection().size() == 0 {
                select_toggle.set_active(false);
            }
            transient.set(false);
        }
    });
    ui.window.add_controller(key_controller);
}

fn connect_bulk_actions(
    ui: Rc<LibraryWindowUi>,
    delete_btn: gtk::Button,
    download_btn: gtk::Button,
    clear_btn: gtk::Button,
) {
    clear_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            ui.grid.selection.unselect_all();
            ui.select_toggle.set_active(false);
        }
    ));

    download_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            let downloads: Vec<(String, String)> = collect_selected_assets(&ui)
                .into_iter()
                .filter(|(asset_id, _)| !asset_id.starts_with(LOCAL_ID_PREFIX))
                .collect();
            if !downloads.is_empty() {
                start_download_group(ui.clone(), downloads);
            }
        }
    ));

    delete_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            confirm_and_bulk_delete(ui.clone());
        }
    ));
}

fn selection_count(selection: &gtk::MultiSelection) -> u32 {
    let bitset = selection.selection();
    bitset.size() as u32
}

fn collect_selected_assets(ui: &Rc<LibraryWindowUi>) -> Vec<(String, String)> {
    let bitset = ui.grid.selection.selection();
    let mut out = Vec::new();
    let Some((mut iter, first)) = gtk::BitsetIter::init_first(&bitset) else {
        return out;
    };
    let mut pos = Some(first);
    while let Some(p) = pos {
        if let Some(item) = ui.grid.model.item(p).and_downcast::<AssetObject>() {
            out.push((
                item.property::<String>("id"),
                item.property::<String>("filename"),
            ));
        }
        pos = iter.next();
    }
    out
}

fn should_refresh_after_download(ui: &LibraryWindowUi) -> bool {
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

fn confirm_and_bulk_delete(ui: Rc<LibraryWindowUi>) {
    let assets = collect_selected_assets(&ui);
    let remote_ids: Vec<String> = assets
        .iter()
        .filter(|(id, _)| !id.starts_with(LOCAL_ID_PREFIX))
        .map(|(id, _)| id.clone())
        .collect();
    if remote_ids.is_empty() {
        return;
    }

    let dialog = libadwaita::AlertDialog::builder()
        .heading(format!("Move {} item(s) to trash?", remote_ids.len()))
        .body("Items can be restored from the Immich trash.")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Move to trash");
    dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let ui_for_choice = ui.clone();
    dialog.connect_response(None, move |dlg, response| {
        if response != "delete" {
            return;
        }
        let ui = ui_for_choice.clone();
        let ids = remote_ids.clone();
        glib::MainContext::default().spawn_local(async move {
            match ui.ctx.api_client.delete_assets(&ids).await {
                Ok(()) => {
                    ui.grid.selection.unselect_all();
                    ui.select_toggle.set_active(false);
                    refresh_library_after_mutation(ui.clone(), true);
                }
                Err(err) => {
                    log::error!("Bulk delete failed: {}", err);
                }
            }
        });
        dlg.close();
    });
    dialog.present(Some(&ui.window));
}

fn connect_sidebar_handlers(ui: Rc<LibraryWindowUi>) {
    // Photos / Explore (fixed destinations).
    ui.sidebar.fixed_list.connect_row_selected(clone!(
        #[strong]
        ui,
        move |_, row| {
            let Some(row) = row else {
                return;
            };
            let key = row.tooltip_text().unwrap_or_default();
            ui.sidebar.albums_list.unselect_all();
            match key.as_str() {
                "photos" => sidebar_dispatch(ui.clone(), LibrarySource::Timeline),
                "explore" => sidebar_dispatch(ui.clone(), LibrarySource::Explore),
                "albums" => {
                    ui.album_link_row.set_visible(false);
                    if let Some(parent) = ui.album_link_row.parent() {
                        parent.set_visible(false);
                    }
                    ui.content_stack.set_visible_child_name("albums");
                    refresh_albums_view(ui.clone());
                }
                _ => {}
            }
        }
    ));

    ui.sidebar.albums_list.connect_row_selected(clone!(
        #[strong]
        ui,
        move |_, row| {
            if ui.sidebar_suppressed.get() {
                return;
            }
            let Some(row) = row else {
                return;
            };
            let Some(tooltip) = row.tooltip_text() else {
                return;
            };
            let mut parts = tooltip.splitn(2, ':');
            let id = parts.next().unwrap_or_default().to_string();
            let name = parts.next().unwrap_or("Album").to_string();
            ui.sidebar.fixed_list.unselect_all();
            sidebar_dispatch(ui.clone(), LibrarySource::Album { id, name });
        }
    ));
}

fn update_back_button(ui: &Rc<LibraryWindowUi>) {
    let can_go_back = ui.ctx.library_state.lock().can_go_back();
    ui.back_button.set_sensitive(can_go_back);
}

/// Pop the navigation history and switch to the previous source. Returns
/// true if a back-step was taken, false when the history was empty.
fn navigate_back(ui: Rc<LibraryWindowUi>) -> bool {
    ui.search_entry.set_text("");
    let request = ui.ctx.library_state.lock().navigate_back();
    if let Some(request) = request {
        apply_timeline_ui_state(&ui, &request.1);
        load_source_page(ui.clone(), request, false);
        update_back_button(&ui);
        true
    } else {
        false
    }
}

/// Common path for sidebar selections. Skips redundant dispatches so the
/// `reload_sidebar` programmatic re-selection doesn't loop into another
/// fetch — but only when the content stack is already showing the current
/// source, since the Albums grid moves the stack without changing source.
fn sidebar_dispatch(ui: Rc<LibraryWindowUi>, source: LibrarySource) {
    let on_albums_grid = ui.content_stack.visible_child_name().as_deref() == Some("albums");
    if !on_albums_grid && ui.ctx.library_state.lock().source == source {
        return;
    }
    let request = ui.ctx.library_state.lock().navigate_to(source);
    apply_timeline_ui_state(&ui, &request.1);
    load_source_page(ui.clone(), request, false);
    update_back_button(&ui);
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
            let local_path = item.property::<String>("local-path");
            let asset_type = item.property::<String>("asset-type");

            // Videos open in the system default player per spec — no in-app
            // playback for v1.
            if asset_type.eq_ignore_ascii_case("VIDEO") {
                if !local_path.is_empty() {
                    open_local_with_default_app(&local_path);
                } else {
                    spawn_video_handoff(ui.clone(), asset_id, filename);
                }
                return;
            }

            open_lightbox(ui.clone(), position);
        }
    ));

    let scroll_pending = Rc::new(std::cell::Cell::new(false));
    ui.grid.scrolled.vadjustment().connect_value_changed(clone!(
        #[strong]
        ui,
        #[strong]
        scroll_pending,
        move |_adj| {
            if scroll_pending.replace(true) {
                return;
            }
            let ui = ui.clone();
            let scroll_pending = scroll_pending.clone();
            glib::idle_add_local_once(move || {
                scroll_pending.set(false);
                let adj = ui.grid.scrolled.vadjustment();
                update_timeline_banner_if_active(&ui, &adj);

                let threshold = (adj.upper() - adj.page_size()) * 0.50;
                if adj.value() < threshold {
                    return;
                }

                let next = ui.ctx.library_state.lock().load_next_page_if_needed();
                if let Some(request) = next {
                    load_source_page(ui.clone(), request, true);
                }
            });
        }
    ));
}

fn show_asset_context_menu(
    ui: Rc<LibraryWindowUi>,
    parent: &impl gtk::prelude::IsA<gtk::Widget>,
    position: u32,
    x: f64,
    y: f64,
) {
    let Some(item) = ui.grid.model.item(position).and_downcast::<AssetObject>() else {
        return;
    };
    let asset_id = item.property::<String>("id");
    let remote_id = item.property::<String>("remote-id");
    let local_path = item.property::<String>("local-path");
    let filename = item.property::<String>("filename");
    let asset_type = item.property::<String>("asset-type");
    let is_image = asset_type.eq_ignore_ascii_case("IMAGE");
    let can_download = !remote_id.is_empty() && !asset_id.starts_with(LOCAL_ID_PREFIX);
    let can_open = can_download || !local_path.is_empty();

    let popover = gtk::Popover::builder()
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    if is_image {
        let copy_btn = gtk::Button::builder()
            .label("Copy")
            .halign(gtk::Align::Fill)
            .build();
        copy_btn.connect_clicked(clone!(
            #[strong]
            ui,
            #[strong]
            popover,
            #[strong]
            asset_id,
            #[strong]
            remote_id,
            #[strong]
            local_path,
            #[strong]
            filename,
            move |_| {
                popover.popdown();
                copy_asset_to_clipboard(
                    ui.clone(),
                    asset_id.clone(),
                    remote_id.clone(),
                    local_path.clone(),
                    filename.clone(),
                );
            }
        ));
        content.append(&copy_btn);
    }

    if can_download {
        let download_btn = gtk::Button::builder()
            .label("Download")
            .halign(gtk::Align::Fill)
            .build();
        download_btn.connect_clicked(clone!(
            #[strong]
            ui,
            #[strong]
            popover,
            #[strong]
            remote_id,
            #[strong]
            filename,
            move |_| {
                popover.popdown();
                start_download(ui.clone(), remote_id.clone(), filename.clone());
            }
        ));
        content.append(&download_btn);
    }

    if can_open {
        let open_btn = gtk::Button::builder()
            .label("Open In")
            .halign(gtk::Align::Fill)
            .build();
        open_btn.connect_clicked(clone!(
            #[strong]
            ui,
            #[strong]
            popover,
            #[strong]
            asset_id,
            #[strong]
            remote_id,
            #[strong]
            local_path,
            #[strong]
            filename,
            move |_| {
                popover.popdown();
                open_asset_in_default_app(
                    ui.clone(),
                    asset_id.clone(),
                    remote_id.clone(),
                    local_path.clone(),
                    filename.clone(),
                );
            }
        ));
        content.append(&open_btn);
    }

    popover.set_child(Some(&content));
    popover.popup();
}

fn copy_asset_to_clipboard(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    remote_id: String,
    local_path: String,
    filename: String,
) {
    glib::MainContext::default().spawn_local(async move {
        let path =
            match ensure_original_asset_path(&ui, &asset_id, &remote_id, &local_path, &filename)
                .await
            {
                Ok(path) => path,
                Err(err) => {
                    show_alert_dialog(&ui, "Copy Failed", &err);
                    return;
                }
            };
        let Some(texture) = load_texture_oriented(&path) else {
            show_alert_dialog(&ui, "Copy Failed", "Could not decode the original image.");
            return;
        };
        if let Some(display) = gdk4::Display::default() {
            display.clipboard().set_texture(&texture);
        }
    });
}

fn open_asset_in_default_app(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    remote_id: String,
    local_path: String,
    filename: String,
) {
    glib::MainContext::default().spawn_local(async move {
        let path =
            match ensure_original_asset_path(&ui, &asset_id, &remote_id, &local_path, &filename)
                .await
            {
                Ok(path) => path,
                Err(err) => {
                    show_alert_dialog(&ui, "Open Failed", &err);
                    return;
                }
            };
        open_local_with_default_app(&path.display().to_string());
    });
}

async fn ensure_original_asset_path(
    ui: &LibraryWindowUi,
    asset_id: &str,
    remote_id: &str,
    local_path: &str,
    filename: &str,
) -> Result<PathBuf, String> {
    if !local_path.is_empty() {
        return Ok(PathBuf::from(local_path));
    }
    let remote_asset_id = if !remote_id.is_empty() {
        remote_id
    } else {
        asset_id
    };
    let cache_dir = crate::profile::cache_dir()
        .ok_or_else(|| "Could not locate a cache directory.".to_string())?
        .join("open-in");
    let _ = std::fs::create_dir_all(&cache_dir);
    let safe_name =
        crate::sanitize::safe_filename(filename).unwrap_or_else(|| asset_id.to_string());
    let path = cache_dir.join(&safe_name);
    if path.exists() {
        return Ok(path);
    }
    begin_download_session(&ui.ctx, filename.to_string());
    let progress = track_download_item(
        &ui.ctx,
        remote_asset_id.to_string(),
        Some(filename.to_string()),
        None,
    );
    let result = ui
        .ctx
        .api_client
        .download_original_to_file(remote_asset_id, &path, Some(progress))
        .await;
    finish_download_item(&ui.ctx, remote_asset_id);
    result.map(|_| path).map_err(|err| err.to_string())
}

fn show_alert_dialog(ui: &LibraryWindowUi, heading: &str, body: &str) {
    let alert = libadwaita::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    alert.add_response("ok", "OK");
    alert.present(Some(&ui.window));
}

fn apply_timeline_ui_state(ui: &LibraryWindowUi, source: &LibrarySource) {
    let timeline_allowed = matches!(source, LibrarySource::AllAssets | LibrarySource::Timeline);
    let timeline_active = matches!(source, LibrarySource::Timeline);
    ui.ctx
        .library_timeline_active
        .store(timeline_active, std::sync::atomic::Ordering::Relaxed);
    ui.timeline_toggle.set_sensitive(timeline_allowed);
    if ui.timeline_toggle.is_active() != timeline_active {
        ui.timeline_toggle.set_active(timeline_active);
    }
    ui.timeline_banner.set_visible(timeline_active);
    if timeline_active {
        ui.sort_mode.set_selected(0);
    }

    let is_local = matches!(
        source,
        LibrarySource::LocalAll
            | LibrarySource::LocalSearch { .. }
            | LibrarySource::AlbumLocal { .. }
    );
    let is_unified = matches!(
        source,
        LibrarySource::Unified
            | LibrarySource::UnifiedSearch { .. }
            | LibrarySource::AlbumUnified { .. }
    );
    let remote_search_allowed = !is_local && !is_unified;
    ui.search_mode.set_visible(remote_search_allowed);
    ui.filters_button.set_visible(remote_search_allowed);
    if !remote_search_allowed {
        ui.search_mode.set_selected(0);
    }

    // Keep source dropdown visually consistent with the active source so
    // sidebar selections don't leave it showing the wrong tab.
    let target = if is_local {
        1
    } else if is_unified {
        2
    } else {
        0
    };
    if ui.source_mode.selected() != target {
        ui.source_mode_suppressed.set(true);
        ui.source_mode.set_selected(target);
        ui.source_mode_suppressed.set(false);
    }

    refresh_album_link_row(ui, source);
}

fn refresh_album_link_row(ui: &LibraryWindowUi, source: &LibrarySource) {
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
            ui.album_link_button.set_label("Link folder…");
            ui.album_sync_button.set_visible(false);
        }
    }
}

fn connect_album_link_row(ui: Rc<LibraryWindowUi>, _listbox: gtk::ListBox) {
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

    let ui_for_async = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        let diff = match crate::library::album_sync::diff_album_vs_folder(
            ui_for_async.ctx.clone(),
            &album_id,
            &watch_path,
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

    if upload_count == 0 && download_count == 0 {
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
    body_box.append(&upload_check);
    body_box.append(&download_check);
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
        if !do_upload && !do_download {
            dlg.close();
            return;
        }
        let ui = ui_for_apply.clone();
        let album_id = album_id.clone();
        let album_name = album_name.clone();
        let watch_path = watch_path.clone();
        let to_upload = if do_upload {
            diff.to_upload.clone()
        } else {
            Vec::new()
        };
        let to_download = if do_download {
            diff.to_download.clone()
        } else {
            Vec::new()
        };
        glib::MainContext::default().spawn_local(async move {
            let queued = if !to_upload.is_empty() {
                crate::library::album_sync::execute_uploads(
                    ui.ctx.clone(),
                    album_id,
                    album_name,
                    watch_path.clone(),
                    to_upload,
                )
                .await
            } else {
                0
            };
            let (downloaded, failed) = if !to_download.is_empty() {
                crate::library::album_sync::execute_downloads(
                    ui.ctx.clone(),
                    watch_path,
                    to_download,
                )
                .await
            } else {
                (0, 0)
            };
            log::info!(
                "Album sync done: {} queued for upload, {} downloaded, {} download failures",
                queued,
                downloaded,
                failed
            );
            if queued > 0 || downloaded > 0 {
                refresh_library_after_mutation(ui.clone(), true);
            }
        });
        dlg.close();
    });
    dialog.present(Some(&ui.window));
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

fn update_timeline_banner_if_active(ui: &Rc<LibraryWindowUi>, adj: &gtk::Adjustment) {
    let state = ui.ctx.library_state.lock();
    if !matches!(state.source, LibrarySource::Timeline) {
        return;
    }
    if state.assets.is_empty() {
        ui.timeline_banner.set_label("");
        return;
    }

    let max = (adj.upper() - adj.page_size()).max(1.0);
    let frac = (adj.value() / max).clamp(0.0, 1.0);
    let idx = ((state.assets.len() as f64) * frac) as usize;
    let idx = idx.min(state.assets.len() - 1);
    let label = month_year_label(&state.assets[idx].created_at);
    ui.timeline_banner.set_label(&label);
}

fn month_year_label(iso: &str) -> String {
    use chrono::{DateTime, Datelike};
    if let Ok(dt) = DateTime::parse_from_rfc3339(iso) {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        let m = dt.month0() as usize;
        if let Some(name) = MONTHS.get(m) {
            return format!("{} {}", name, dt.year());
        }
    }
    iso.chars().take(7).collect()
}

fn load_albums(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            match ui.ctx.api_client.fetch_library_albums().await {
                Ok(albums) => {
                    ui.ctx.library_state.lock().load_albums(albums);
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
                let mut state = ui.ctx.library_state.lock();
                state.set_status(stats, about);
            }
            update_footer(&ui, route);
        }
    ));
}

fn load_explore_landing(ui: Rc<LibraryWindowUi>) {
    ui.content_stack.set_visible_child_name("explore");
    if ui.explore.populated.get() {
        return;
    }
    ui.explore.populated.set(true);
    let ctx = ui.ctx.clone();

    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let people = ctx.api_client.fetch_people().await.unwrap_or_default();
            let sections = ctx.api_client.fetch_explore().await.unwrap_or_default();
            let places = ctx.api_client.fetch_all_places().await.unwrap_or_default();

            let click_ui = ui.clone();
            explore_view::populate_people(&ui.explore, ctx.clone(), people, move |id, name| {
                let filters = MetadataSearchFilters {
                    person_ids: Some(vec![id]),
                    ..Default::default()
                };
                let request = click_ui.ctx.library_state.lock().switch_source(
                    LibrarySource::AdvancedSearch {
                        filters: Box::new(filters),
                    },
                );
                click_ui.search_entry.set_text(&name);
                apply_timeline_ui_state(&click_ui, &request.1);
                load_source_page(click_ui.clone(), request, false);
            });
            let click_ui = ui.clone();
            explore_view::populate_places(&ui.explore, ctx.clone(), places, move |_kind, value| {
                let next = LibrarySource::AdvancedSearch {
                    filters: Box::new(MetadataSearchFilters {
                        city: Some(value.clone()),
                        ..Default::default()
                    }),
                };
                let request = click_ui.ctx.library_state.lock().switch_source(next);
                click_ui.search_entry.set_text(&value);
                apply_timeline_ui_state(&click_ui, &request.1);
                load_source_page(click_ui.clone(), request, false);
            });
            let click_ui = ui.clone();
            explore_view::populate_explore(
                &ui.explore,
                ctx.clone(),
                sections,
                move |_kind, value| {
                    let next = LibrarySource::SmartSearch {
                        query: value.clone(),
                    };
                    let request = click_ui.ctx.library_state.lock().switch_source(next);
                    click_ui.search_entry.set_text(&value);
                    apply_timeline_ui_state(&click_ui, &request.1);
                    load_source_page(click_ui.clone(), request, false);
                },
            );
        }
    ));
}

fn load_source_page(ui: Rc<LibraryWindowUi>, request: (u64, LibrarySource, u32), append: bool) {
    if matches!(request.1, LibrarySource::Explore) {
        load_explore_landing(ui);
        return;
    }
    if !append {
        ui.content_stack.set_visible_child_name("loading");
    }
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let (generation, source, page) = request;
            let order = ui.ctx.library_state.lock().sort_mode.server_order();
            let result: Result<(Vec<LibraryAsset>, bool), String> = match source.clone() {
                LibrarySource::AllAssets | LibrarySource::Timeline => {
                    ui.ctx
                        .api_client
                        .search_metadata("", page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::Explore => unreachable!("intercepted above"),
                LibrarySource::Album { id, .. } => {
                    ui.ctx
                        .api_client
                        .fetch_album_assets(&id, page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::SmartSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_smart(&query, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::OcrSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_ocr(&query, page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::MetadataSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE, order)
                        .await
                }
                LibrarySource::AdvancedSearch { filters } => {
                    let mut filters = (*filters).clone();
                    filters.order = order;
                    ui.ctx
                        .api_client
                        .search_metadata_with_filters(&filters, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::LocalAll => {
                    // Local enumeration is bounded — single synthetic page.
                    if page > 1 {
                        Ok((Vec::new(), false))
                    } else {
                        let locals = enumerate_local(ui.ctx.clone()).await;
                        Ok((
                            locals.into_iter().map(local_to_library_asset).collect(),
                            false,
                        ))
                    }
                }
                LibrarySource::LocalSearch { query } => {
                    if page > 1 {
                        Ok((Vec::new(), false))
                    } else {
                        let locals = enumerate_local(ui.ctx.clone()).await;
                        let filtered = filter_by_filename(locals, &query);
                        Ok((
                            filtered.into_iter().map(local_to_library_asset).collect(),
                            false,
                        ))
                    }
                }
                LibrarySource::Unified => {
                    let remote = ui
                        .ctx
                        .api_client
                        .search_metadata("", page, PAGE_SIZE, order)
                        .await;
                    merge_unified_page(remote, page, &ui, None).await
                }
                LibrarySource::UnifiedSearch { query } => {
                    let remote = ui
                        .ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE, order)
                        .await;
                    merge_unified_page(remote, page, &ui, Some(&query)).await
                }
                LibrarySource::AlbumLocal { name, .. } => {
                    if page > 1 {
                        Ok((Vec::new(), false))
                    } else {
                        match linked_entry_path_for_album(&ui, &name) {
                            Some(path) => {
                                let locals = enumerate_local_for_entry(ui.ctx.clone(), path).await;
                                Ok((
                                    locals.into_iter().map(local_to_library_asset).collect(),
                                    false,
                                ))
                            }
                            None => Ok((Vec::new(), false)),
                        }
                    }
                }
                LibrarySource::AlbumUnified { id, name } => {
                    let remote = ui
                        .ctx
                        .api_client
                        .fetch_album_assets(&id, page, PAGE_SIZE, order)
                        .await;
                    merge_album_unified_page(remote, page, &ui, &name).await
                }
            };

            match result {
                Ok((items, has_more)) => {
                    {
                        let mut state = ui.ctx.library_state.lock();
                        let applied = if append {
                            state.append_assets_with_more(generation, items, has_more)
                        } else {
                            state.replace_assets_with_more(generation, items, has_more)
                        };
                        if !applied {
                            return;
                        }
                        if append {
                            ui.grid
                                .model
                                .extend(&ui.ctx, &state.assets, &state.sort_mode);
                        } else {
                            ui.grid
                                .model
                                .reset(&ui.ctx, &state.assets, &state.sort_mode);
                        }
                    }
                    sync_content_state(&ui);
                    reload_sidebar(&ui);
                    update_timeline_banner_if_active(&ui, &ui.grid.scrolled.vadjustment());
                }
                Err(err) => {
                    let mut state = ui.ctx.library_state.lock();
                    state.mark_error(generation, err.clone());
                    ui.error_label
                        .set_label(&format!("Could not load library assets: {}", err));
                    ui.content_stack.set_visible_child_name("error");
                }
            }
        }
    ));
}

fn reload_sidebar(ui: &Rc<LibraryWindowUi>) {
    while let Some(row) = ui.sidebar.albums_list.first_child() {
        ui.sidebar.albums_list.remove(&row);
    }

    let selected_source = ui.ctx.library_state.lock().source.clone();
    let albums = ui.ctx.library_state.lock().albums.clone();
    for album in albums {
        let subtitle = format!("{} asset(s)", album.asset_count);
        let action = libadwaita::ActionRow::builder()
            .title(&album.album_name)
            .subtitle(&subtitle)
            .build();
        let row = gtk::ListBoxRow::builder()
            .tooltip_text(format!("{}:{}", album.id, album.album_name))
            .child(&action)
            .build();
        ui.sidebar.albums_list.append(&row);
    }

    match selected_source {
        LibrarySource::Timeline => {
            select_fixed_row(&ui.sidebar.fixed_list, "photos");
            ui.sidebar.albums_list.unselect_all();
        }
        LibrarySource::Explore => {
            select_fixed_row(&ui.sidebar.fixed_list, "explore");
            ui.sidebar.albums_list.unselect_all();
        }
        LibrarySource::Album { id, .. }
        | LibrarySource::AlbumLocal { id, .. }
        | LibrarySource::AlbumUnified { id, .. } => {
            ui.sidebar.fixed_list.unselect_all();
            ui.sidebar_suppressed.set(true);
            let mut child = ui.sidebar.albums_list.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
                    && row.tooltip_text().as_deref().is_some_and(|tooltip| {
                        tooltip.split_once(':').map(|(prefix, _)| prefix) == Some(id.as_str())
                    })
                {
                    ui.sidebar.albums_list.select_row(Some(&row));
                    ui.sidebar_suppressed.set(false);
                    break;
                }
                child = next;
            }
            ui.sidebar_suppressed.set(false);
        }
        _ => {
            ui.sidebar.fixed_list.unselect_all();
            ui.sidebar.albums_list.unselect_all();
        }
    }
}

fn select_fixed_row(list: &gtk::ListBox, key: &str) {
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
            && row.tooltip_text().as_deref() == Some(key)
        {
            list.select_row(Some(&row));
            return;
        }
        child = next;
    }
}

fn sync_content_state(ui: &LibraryWindowUi) {
    match &ui.ctx.library_state.lock().load_state {
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
    let state = ui.ctx.library_state.lock();
    let route_subtitle = route
        .as_deref()
        .map(|route| match route {
            "LAN" => "Connected through LAN",
            "WAN" => "Connected through WAN",
            _ => "Connected through configured server",
        })
        .unwrap_or("Offline");
    ui.sidebar.connection_row.set_subtitle(route_subtitle);

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
    ui.sidebar
        .server_row
        .set_subtitle(&format!("{stats} | {about}"));
}

fn update_transfer_ui(ui: &LibraryWindowUi) {
    let transfer = {
        let mut state = ui.ctx.state.lock();
        if state.transfer.active
            && state.transfer.active_uploads == 0
            && state.transfer.active_downloads == 0
        {
            // Guard against sessions that were opened but never queued.
            state.transfer.reset_runtime();
        }
        state.transfer.clone()
    };
    if !transfer.active {
        ui.transfer_bar.remove_css_class("active");
        ui.transfer_progress.set_fraction(0.0);
        ui.transfer_icon.set_visible(false);
        let idle_summary =
            if transfer.last_upload_avg_bps > 0.0 || transfer.last_download_avg_bps > 0.0 {
                format!(
                    "Idle  Last upload avg {}  Last download avg {}",
                    format_rate(transfer.last_upload_avg_bps),
                    format_rate(transfer.last_download_avg_bps)
                )
            } else {
                "Idle  No recent transfer session".to_string()
            };
        ui.transfer_label.set_label(&idle_summary);
        return;
    }
    ui.transfer_bar.add_css_class("active");

    let icon_name = match transfer.direction {
        TransferDirection::Upload => "mimick-upload-symbolic",
        TransferDirection::Download => "mimick-download-symbolic",
    };
    ui.transfer_icon.set_icon_name(Some(icon_name));
    ui.transfer_icon.set_visible(true);

    let detail = transfer
        .active_item_label
        .as_deref()
        .unwrap_or(match transfer.direction {
            TransferDirection::Upload => "queued asset",
            TransferDirection::Download => "selected asset",
        });
    let live_speed = format_rate(transfer.instant_bps);
    let avg_speed = format_rate(transfer.session_avg_bps);
    ui.transfer_label
        .set_label(&format!("{detail}  {live_speed}  avg {avg_speed}"));

    match transfer.total_bytes {
        Some(total) if total > 0 => {
            ui.transfer_progress.set_show_text(false);
            ui.transfer_progress
                .set_fraction((transfer.current_bytes as f64 / total as f64).clamp(0.0, 1.0));
        }
        _ => {
            ui.transfer_progress.pulse();
        }
    }
}

fn format_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec.max(0.0))
    }
}

fn build_status_view(icon_name: &str, title: &str, subtitle: &str) -> gtk::Box {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .vexpand(true)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .css_classes(vec!["mimick-empty".to_string()])
        .build();
    let icon = gtk::Image::builder()
        .icon_name(icon_name)
        .pixel_size(64)
        .build();
    icon.add_css_class("dim-label");
    let title_label = gtk::Label::builder()
        .label(title)
        .css_classes(vec!["mimick-empty-title".to_string()])
        .build();
    let subtitle_label = gtk::Label::builder()
        .label(subtitle)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(vec!["mimick-empty-subtitle".to_string()])
        .build();
    container.append(&icon);
    container.append(&title_label);
    container.append(&subtitle_label);
    container
}

pub(super) fn immich_checksum_to_hex(b64: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

fn fill_exif_box(container: &gtk::Box, exif: &crate::api_client::ExifInfo) {
    let mut rows: Vec<(String, String)> = Vec::new();
    let dims = match (exif.exif_image_width, exif.exif_image_height) {
        (Some(w), Some(h)) => Some(format!("{} × {}", w, h)),
        _ => None,
    };
    if let Some(d) = dims {
        rows.push(("Dimensions".into(), d));
    }
    if let Some(size) = exif.file_size_in_byte {
        rows.push(("Size".into(), format_bytes(size)));
    }
    if let Some(dt) = &exif.date_time_original {
        rows.push(("Taken".into(), format_datetime_display(dt)));
    }
    let camera = match (&exif.make, &exif.model) {
        (Some(m), Some(n)) => Some(format!("{} {}", m, n)),
        (Some(m), None) => Some(m.clone()),
        (None, Some(n)) => Some(n.clone()),
        _ => None,
    };
    if let Some(c) = camera {
        rows.push(("Camera".into(), c));
    }
    if let Some(l) = &exif.lens_model {
        rows.push(("Lens".into(), l.clone()));
    }
    let mut shot = Vec::new();
    if let Some(f) = exif.f_number {
        shot.push(format!("ƒ/{:.1}", f));
    }
    if let Some(et) = &exif.exposure_time {
        shot.push(et.clone());
    }
    if let Some(iso) = exif.iso {
        shot.push(format!("ISO {}", iso));
    }
    if let Some(focal) = exif.focal_length {
        shot.push(format!("{:.0}mm", focal));
    }
    if !shot.is_empty() {
        rows.push(("Exposure".into(), shot.join(" · ")));
    }
    let location = match (&exif.city, &exif.state, &exif.country) {
        (Some(c), Some(s), Some(co)) => Some(format!("{}, {}, {}", c, s, co)),
        (Some(c), None, Some(co)) => Some(format!("{}, {}", c, co)),
        (Some(c), _, _) => Some(c.clone()),
        (_, Some(s), Some(co)) => Some(format!("{}, {}", s, co)),
        (_, _, Some(co)) => Some(co.clone()),
        _ => None,
    };
    if let Some(loc) = location {
        rows.push(("Location".into(), loc));
    }
    if let (Some(lat), Some(lon)) = (exif.latitude, exif.longitude) {
        rows.push(("GPS".into(), format!("{:.5}, {:.5}", lat, lon)));
    }
    if let Some(desc) = &exif.description
        && !desc.is_empty()
    {
        rows.push(("Description".into(), desc.clone()));
    }

    for (key, value) in rows {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        let k = gtk::Label::builder()
            .label(&key)
            .xalign(0.0)
            .css_classes(vec!["caption".to_string(), "dim-label".to_string()])
            .build();
        let v = gtk::Label::builder()
            .label(&value)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .selectable(true)
            .build();
        row.append(&k);
        row.append(&v);
        container.append(&row);
    }
}

fn format_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    let n_f = n as f64;
    if n_f >= KIB * KIB * KIB {
        format!("{:.2} GB", n_f / (KIB * KIB * KIB))
    } else if n_f >= KIB * KIB {
        format!("{:.2} MB", n_f / (KIB * KIB))
    } else if n_f >= KIB {
        format!("{:.1} KB", n_f / KIB)
    } else {
        format!("{} B", n)
    }
}

/// Format an ISO 8601 timestamp for display, converting from UTC to the
/// user's local timezone.
///
/// Immich normalises `date_time_original` and `fileCreatedAt` to UTC before
/// storage, so a photo taken at 19:55:15+05:30 is stored as
/// 2024-01-15T14:25:15.000Z. We parse the UTC value and convert it to the
/// system's local timezone so the displayed time matches what the camera
/// originally recorded. Falls back to the raw string if parsing fails.
fn format_datetime_display(iso: &str) -> String {
    use chrono::{DateTime, Local, Utc};
    // Try offset-aware parse first (handles +05:30, Z, etc.)
    if let Ok(dt) = DateTime::parse_from_rfc3339(iso) {
        let local: DateTime<Local> = dt.into();
        return local.format("%Y-%m-%d %H:%M:%S UTC%:z").to_string();
    }
    // Fallback: try treating as UTC
    if let Ok(dt) = iso.parse::<DateTime<Utc>>() {
        let local: DateTime<Local> = dt.into();
        return local.format("%Y-%m-%d %H:%M:%S UTC%:z").to_string();
    }
    // Last resort: strip trailing fractional seconds / timezone suffix
    iso.get(..19).unwrap_or(iso).replace('T', " ").to_string()
}

/// Apply zoom to a lightbox Picture. Zoom is fit-relative: 1.0 = the size the
/// texture would occupy inside `viewer` under Contain layout. > 1.0 overflows
/// the viewer for panning. At 1.0 we restore (-1, -1) so the picture
/// auto-resizes with the window.
fn apply_lightbox_zoom(picture: &gtk::Picture, viewer: &gtk::ScrolledWindow, zoom: f64) {
    if (zoom - 1.0).abs() < 0.001 {
        picture.set_size_request(-1, -1);
        return;
    }
    let Some(paintable) = picture.paintable() else {
        picture.set_size_request(-1, -1);
        return;
    };
    let nw = paintable.intrinsic_width().max(1) as f64;
    let nh = paintable.intrinsic_height().max(1) as f64;
    let viewer_w = viewer.width().max(1) as f64;
    let viewer_h = viewer.height().max(1) as f64;
    let texture_aspect = nw / nh;
    let viewer_aspect = viewer_w / viewer_h;
    let (fit_w, fit_h) = if viewer_aspect > texture_aspect {
        (viewer_h * texture_aspect, viewer_h)
    } else {
        (viewer_w, viewer_w / texture_aspect)
    };
    picture.set_size_request((fit_w * zoom) as i32, (fit_h * zoom) as i32);
}

fn open_lightbox(ui: Rc<LibraryWindowUi>, position: u32) {
    let Some(item) = ui.grid.model.item(position).and_downcast::<AssetObject>() else {
        return;
    };
    let initial_filename = item.property::<String>("filename");

    let page = libadwaita::NavigationPage::builder()
        .title(&initial_filename)
        .can_pop(true)
        .build();
    let toolbar = libadwaita::ToolbarView::builder().build();
    let header = libadwaita::HeaderBar::builder()
        .show_back_button(true)
        .build();
    let prev_btn = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Previous (Left)")
        .build();
    let next_btn = gtk::Button::builder()
        .icon_name("go-next-symbolic")
        .tooltip_text("Next (Right)")
        .build();
    let details_btn = gtk::ToggleButton::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Toggle details (I)")
        .active(false)
        .build();
    header.pack_start(&prev_btn);
    header.pack_start(&next_btn);
    header.pack_end(&details_btn);
    toolbar.add_top_bar(&header);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    let viewer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .hexpand(true)
        .build();
    // Two picture widgets in a stack so navigation can slide between them.
    let picture_a = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .vexpand(true)
        .hexpand(true)
        .build();
    let picture_b = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .vexpand(true)
        .hexpand(true)
        .build();
    let pic_stack = gtk::Stack::builder()
        .transition_duration(180)
        .vexpand(true)
        .hexpand(true)
        .build();
    pic_stack.add_named(&picture_a, Some("a"));
    pic_stack.add_named(&picture_b, Some("b"));
    pic_stack.set_visible_child_name("a");
    let scrolled_picture = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&pic_stack)
        .vexpand(true)
        .hexpand(true)
        .build();
    let active_a = Rc::new(Cell::new(true));
    let zoom_level = Rc::new(Cell::new(1.0_f64));
    let initial_full = ui.ctx.config.read().data.library_preview_full_resolution;
    let resolution_toggle = gtk::ToggleButton::builder()
        .label(if initial_full { "Original" } else { "Preview" })
        .tooltip_text("Toggle preview vs original full-resolution image")
        .active(initial_full)
        .build();
    let download = gtk::Button::builder().label("Download").build();
    let zoom_out_btn = gtk::Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text("Zoom out (Ctrl+-)")
        .build();
    let zoom_in_btn = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text("Zoom in (Ctrl++)")
        .build();
    let zoom_reset_btn = gtk::Button::builder()
        .label("100%")
        .tooltip_text("Reset zoom (Ctrl+0)")
        .build();
    let zoom_group = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(vec!["linked".to_string()])
        .build();
    zoom_group.append(&zoom_out_btn);
    zoom_group.append(&zoom_reset_btn);
    zoom_group.append(&zoom_in_btn);
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let actions_spacer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();
    actions.append(&zoom_group);
    actions.append(&actions_spacer);
    actions.append(&resolution_toggle);
    actions.append(&download);
    viewer.append(&scrolled_picture);
    viewer.append(&actions);

    let details_inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let details_pane = gtk::ScrolledWindow::builder()
        .child(&details_inner)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(false)
        .min_content_width(320)
        .max_content_width(320)
        .visible(false)
        .build();
    let details_filename = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(36)
        .css_classes(vec!["title-3".to_string()])
        .build();
    let details_summary = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(36)
        .build();
    let details_loading = gtk::Label::builder()
        .xalign(0.0)
        .label("Loading details…")
        .css_classes(vec!["dim-label".to_string()])
        .build();
    let details_exif = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .visible(false)
        .build();
    details_inner.append(&details_filename);
    details_inner.append(&details_summary);
    details_inner.append(&details_loading);
    details_inner.append(&details_exif);

    body.append(&viewer);
    body.append(&details_pane);
    toolbar.set_content(Some(&body));
    page.set_child(Some(&toolbar));

    details_btn
        .bind_property("active", &details_pane, "visible")
        .sync_create()
        .build();

    let pos_cell = Rc::new(Cell::new(position));
    let load_into_picture = Rc::new({
        let ui = ui.clone();
        move |target: gtk::Picture, asset_id: String, local_path: String, full_res: bool| {
            let ui = ui.clone();
            glib::MainContext::default().spawn_local(async move {
                if !local_path.is_empty() {
                    if let Some(texture) = load_texture_oriented(std::path::Path::new(&local_path))
                    {
                        target.set_paintable(Some(&texture));
                    }
                    return;
                }
                if full_res {
                    if let Some(cache_dir) = crate::profile::cache_dir().map(|p| p.join("preview"))
                    {
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let temp = cache_dir.join(format!("{}.bin", asset_id));
                        if !temp.exists()
                            && let Err(err) = {
                                begin_download_session(&ui.ctx, format!("preview {asset_id}"));
                                let progress = track_download_item(
                                    &ui.ctx,
                                    asset_id.clone(),
                                    Some(format!("preview {asset_id}")),
                                    None,
                                );
                                let result = ui
                                    .ctx
                                    .api_client
                                    .download_original_to_file(&asset_id, &temp, Some(progress))
                                    .await;
                                finish_download_item(&ui.ctx, &asset_id);
                                result
                            }
                        {
                            log::warn!("Lightbox original fetch failed: {}", err);
                            return;
                        }
                        if let Some(texture) = load_texture_oriented(&temp) {
                            target.set_paintable(Some(&texture));
                        }
                    }
                } else if let Ok(texture) = ui
                    .ctx
                    .thumbnail_cache
                    .load_thumbnail(&asset_id, ThumbnailSize::Preview)
                    .await
                {
                    target.set_paintable(Some(&texture));
                }
            });
        }
    });

    // -1 = back/prev (slide right), +1 = forward/next (slide left), 0 = no transition
    let nav_dir = Rc::new(Cell::new(0i8));
    let render = Rc::new({
        let ui = ui.clone();
        let page = page.clone();
        let pos_cell = pos_cell.clone();
        let load_into_picture = load_into_picture.clone();
        let resolution_toggle = resolution_toggle.clone();
        let download = download.clone();
        let prev_btn = prev_btn.clone();
        let next_btn = next_btn.clone();
        let details_filename = details_filename.clone();
        let details_summary = details_summary.clone();
        let details_loading = details_loading.clone();
        let details_exif = details_exif.clone();
        let pic_stack = pic_stack.clone();
        let picture_a = picture_a.clone();
        let picture_b = picture_b.clone();
        let scrolled_picture = scrolled_picture.clone();
        let active_a = active_a.clone();
        let zoom_level = zoom_level.clone();
        let zoom_reset_btn = zoom_reset_btn.clone();
        let nav_dir = nav_dir.clone();
        move || {
            let pos = pos_cell.get();
            let n = ui.grid.model.n_items();
            let Some(item) = ui.grid.model.item(pos).and_downcast::<AssetObject>() else {
                return;
            };
            let asset_id = item.property::<String>("id");
            let filename = item.property::<String>("filename");
            let local_path = item.property::<String>("local-path");
            let mime = item.property::<String>("mime-type");
            let created = item.property::<String>("created-at");
            let sync_state = item.property::<u32>("sync-state");

            page.set_title(&filename);
            details_filename.set_label(&filename);
            let sync_label = match sync_state {
                2 => "On Immich and locally",
                1 => "Local only",
                _ => "On Immich only",
            };
            details_summary.set_label(&format!(
                "{} · {}\nCreated: {}",
                mime,
                sync_label,
                format_datetime_display(&created)
            ));

            while let Some(c) = details_exif.first_child() {
                details_exif.remove(&c);
            }
            details_exif.set_visible(false);

            prev_btn.set_sensitive(pos > 0);
            next_btn.set_sensitive(pos + 1 < n);

            let is_local = !local_path.is_empty() && asset_id.starts_with(LOCAL_ID_PREFIX);
            resolution_toggle.set_visible(!is_local);
            download.set_visible(!is_local);

            // Pick the *inactive* picture to load into, then transition to it.
            let target_is_a = !active_a.get();
            let target = if target_is_a {
                picture_a.clone()
            } else {
                picture_b.clone()
            };
            zoom_level.set(1.0);
            apply_lightbox_zoom(&target, &scrolled_picture, 1.0);
            zoom_reset_btn.set_label("100%");
            pic_stack.set_transition_type(match nav_dir.get() {
                1 => gtk::StackTransitionType::SlideLeft,
                -1 => gtk::StackTransitionType::SlideRight,
                _ => gtk::StackTransitionType::None,
            });
            (*load_into_picture)(
                target,
                asset_id.clone(),
                local_path,
                resolution_toggle.is_active(),
            );
            pic_stack.set_visible_child_name(if target_is_a { "a" } else { "b" });
            active_a.set(target_is_a);
            nav_dir.set(0);

            if is_local {
                details_loading.set_visible(false);
                return;
            }

            details_loading.set_visible(true);
            let pos_cell_async = pos_cell.clone();
            let ui_async = ui.clone();
            let details_loading = details_loading.clone();
            let details_exif = details_exif.clone();
            let asset_id_async = asset_id.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = ui_async
                    .ctx
                    .api_client
                    .fetch_asset_details(&asset_id_async)
                    .await;
                if pos_cell_async.get() != pos {
                    return;
                }
                details_loading.set_visible(false);
                let Ok(details) = result else { return };
                if let Some(exif) = details.exif_info {
                    fill_exif_box(&details_exif, &exif);
                    details_exif.set_visible(true);
                }
            });
        }
    });

    (*render)();

    prev_btn.connect_clicked(clone!(
        #[strong]
        pos_cell,
        #[strong]
        render,
        #[strong]
        nav_dir,
        move |_| {
            let pos = pos_cell.get();
            if pos > 0 {
                pos_cell.set(pos - 1);
                nav_dir.set(-1);
                (*render)();
            }
        }
    ));
    let goto_next = Rc::new(clone!(
        #[strong]
        ui,
        #[strong]
        pos_cell,
        #[strong]
        render,
        #[strong]
        next_btn,
        #[strong]
        nav_dir,
        move || {
            let pos = pos_cell.get();
            if pos + 1 < ui.grid.model.n_items() {
                pos_cell.set(pos + 1);
                nav_dir.set(1);
                (*render)();
                return;
            }
            let next_request = ui.ctx.library_state.lock().load_next_page_if_needed();
            let Some(req) = next_request else {
                return;
            };
            next_btn.set_sensitive(false);
            let model = ui.grid.model.clone();
            let pos_cell_h = pos_cell.clone();
            let render_h = render.clone();
            let next_btn_h = next_btn.clone();
            let nav_dir_h = nav_dir.clone();
            let prev_count = model.n_items();
            let handler_id = Rc::new(std::cell::RefCell::new(None::<glib::SignalHandlerId>));
            let handler_id_clone = handler_id.clone();
            let id = model.connect_items_changed(move |m, _, _, _| {
                if m.n_items() <= prev_count {
                    return;
                }
                let pos = pos_cell_h.get();
                if pos + 1 < m.n_items() {
                    pos_cell_h.set(pos + 1);
                    nav_dir_h.set(1);
                    (*render_h)();
                }
                next_btn_h.set_sensitive(true);
                if let Some(hid) = handler_id_clone.borrow_mut().take() {
                    m.disconnect(hid);
                }
            });
            *handler_id.borrow_mut() = Some(id);
            load_source_page(ui.clone(), req, true);
        }
    ));

    next_btn.connect_clicked(clone!(
        #[strong]
        goto_next,
        move |_| (*goto_next)()
    ));

    let active_picture = clone!(
        #[strong]
        active_a,
        #[strong]
        picture_a,
        #[strong]
        picture_b,
        move || -> gtk::Picture {
            if active_a.get() {
                picture_a.clone()
            } else {
                picture_b.clone()
            }
        }
    );

    // Track cursor position over the picture area so zoom can be focal-point
    // aware. None when the cursor is outside the viewer; falls back to centre.
    let cursor_pos: Rc<Cell<Option<(f64, f64)>>> = Rc::new(Cell::new(None));
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(clone!(
        #[strong]
        cursor_pos,
        move |_, x, y| {
            cursor_pos.set(Some((x, y)));
        }
    ));
    motion.connect_leave(clone!(
        #[strong]
        cursor_pos,
        move |_| {
            cursor_pos.set(None);
        }
    ));
    scrolled_picture.add_controller(motion);

    let set_zoom = Rc::new(clone!(
        #[strong]
        zoom_level,
        #[strong]
        active_picture,
        #[strong]
        scrolled_picture,
        #[strong]
        zoom_reset_btn,
        #[strong]
        cursor_pos,
        move |z: f64| {
            let z_new = z.clamp(1.0, 10.0);
            let z_old = zoom_level.get();
            if (z_new - z_old).abs() < 0.0001 {
                zoom_reset_btn.set_label(&format!("{}%", (z_new * 100.0).round() as i32));
                return;
            }

            // Pick the focal point: cursor if inside the viewer, else centre.
            let viewer_w = scrolled_picture.width().max(1) as f64;
            let viewer_h = scrolled_picture.height().max(1) as f64;
            let (fx, fy) = cursor_pos
                .get()
                .filter(|&(x, y)| x >= 0.0 && y >= 0.0 && x <= viewer_w && y <= viewer_h)
                .unwrap_or((viewer_w / 2.0, viewer_h / 2.0));

            let hadj = scrolled_picture.hadjustment();
            let vadj = scrolled_picture.vadjustment();
            let scroll_x = hadj.value();
            let scroll_y = vadj.value();
            let ratio = z_new / z_old.max(0.0001);
            let target_scroll_x = (scroll_x + fx) * ratio - fx;
            let target_scroll_y = (scroll_y + fy) * ratio - fy;

            zoom_level.set(z_new);
            apply_lightbox_zoom(&active_picture(), &scrolled_picture, z_new);
            zoom_reset_btn.set_label(&format!("{}%", (z_new * 100.0).round() as i32));

            // Defer scroll-position update until after layout has run, so the
            // ScrolledWindow's adjustment ranges reflect the new picture size.
            glib::idle_add_local_once(move || {
                hadj.set_value(target_scroll_x);
                vadj.set_value(target_scroll_y);
            });
        }
    ));

    let zoom_by = Rc::new(clone!(
        #[strong]
        zoom_level,
        #[strong]
        set_zoom,
        move |factor: f64| {
            (*set_zoom)(zoom_level.get() * factor);
        }
    ));

    let zoom_reset = Rc::new(clone!(
        #[strong]
        set_zoom,
        move || {
            (*set_zoom)(1.0);
        }
    ));

    zoom_in_btn.connect_clicked(clone!(
        #[strong]
        zoom_by,
        move |_| (*zoom_by)(1.2)
    ));
    zoom_out_btn.connect_clicked(clone!(
        #[strong]
        zoom_by,
        move |_| (*zoom_by)(1.0 / 1.2)
    ));
    zoom_reset_btn.connect_clicked(clone!(
        #[strong]
        zoom_reset,
        move |_| (*zoom_reset)()
    ));

    // Trackpad pinch-to-zoom.
    let pinch = gtk::GestureZoom::new();
    let pinch_start = Rc::new(Cell::new(1.0_f64));
    pinch.connect_begin(clone!(
        #[strong]
        zoom_level,
        #[strong]
        pinch_start,
        move |_, _| {
            pinch_start.set(zoom_level.get());
        }
    ));
    pinch.connect_scale_changed(clone!(
        #[strong]
        pinch_start,
        #[strong]
        set_zoom,
        move |_, scale| {
            (*set_zoom)(pinch_start.get() * scale);
        }
    ));
    scrolled_picture.add_controller(pinch);

    // Click-and-drag panning: only acts when zoomed in (otherwise scrollbars
    // have nowhere to scroll to). Snapshots scroll position on begin and
    // applies cumulative offsets on each update.
    let drag_start = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
    let drag = gtk::GestureDrag::new();
    drag.set_button(gtk::gdk::BUTTON_PRIMARY);
    drag.connect_drag_begin(clone!(
        #[strong]
        scrolled_picture,
        #[strong]
        drag_start,
        move |_, _, _| {
            let hadj = scrolled_picture.hadjustment();
            let vadj = scrolled_picture.vadjustment();
            drag_start.set((hadj.value(), vadj.value()));
        }
    ));
    drag.connect_drag_update(clone!(
        #[strong]
        scrolled_picture,
        #[strong]
        drag_start,
        move |_, off_x, off_y| {
            let (sx0, sy0) = drag_start.get();
            scrolled_picture.hadjustment().set_value(sx0 - off_x);
            scrolled_picture.vadjustment().set_value(sy0 - off_y);
        }
    ));
    scrolled_picture.add_controller(drag);

    // Double-click on the picture: zoom in 2x toward the click position.
    let double_click = gtk::GestureClick::new();
    double_click.set_button(gtk::gdk::BUTTON_PRIMARY);
    double_click.connect_pressed(clone!(
        #[strong]
        cursor_pos,
        #[strong]
        zoom_level,
        #[strong]
        set_zoom,
        move |_, n_press, x, y| {
            if n_press == 2 {
                cursor_pos.set(Some((x, y)));
                (*set_zoom)(zoom_level.get() * 2.0);
            }
        }
    ));
    scrolled_picture.add_controller(double_click);

    // Middle-click: reset zoom to 100%.
    let middle_click = gtk::GestureClick::new();
    middle_click.set_button(gtk::gdk::BUTTON_MIDDLE);
    middle_click.connect_pressed(clone!(
        #[strong]
        zoom_reset,
        move |_, _, _, _| {
            (*zoom_reset)();
        }
    ));
    scrolled_picture.add_controller(middle_click);

    // Right-click: open the standard asset context menu.
    let right_click = gtk::GestureClick::new();
    right_click.set_button(gtk::gdk::BUTTON_SECONDARY);
    right_click.connect_pressed(clone!(
        #[strong]
        ui,
        #[strong]
        pos_cell,
        #[strong]
        scrolled_picture,
        move |_, _, x, y| {
            show_asset_context_menu(ui.clone(), &scrolled_picture, pos_cell.get(), x, y);
        }
    ));
    scrolled_picture.add_controller(right_click);

    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed(clone!(
        #[strong]
        ui,
        #[strong]
        pos_cell,
        #[strong]
        render,
        #[strong]
        details_btn,
        #[strong]
        goto_next,
        #[strong]
        nav_dir,
        #[strong]
        zoom_by,
        #[strong]
        zoom_reset,
        move |_, key, _, mods| {
            let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            match (ctrl, key) {
                (true, gtk::gdk::Key::plus)
                | (true, gtk::gdk::Key::equal)
                | (true, gtk::gdk::Key::KP_Add) => {
                    (*zoom_by)(1.2);
                    glib::Propagation::Stop
                }
                (true, gtk::gdk::Key::minus) | (true, gtk::gdk::Key::KP_Subtract) => {
                    (*zoom_by)(1.0 / 1.2);
                    glib::Propagation::Stop
                }
                (true, gtk::gdk::Key::_0) | (true, gtk::gdk::Key::KP_0) => {
                    (*zoom_reset)();
                    glib::Propagation::Stop
                }
                (false, gtk::gdk::Key::Left) => {
                    let pos = pos_cell.get();
                    if pos > 0 {
                        pos_cell.set(pos - 1);
                        nav_dir.set(-1);
                        (*render)();
                    }
                    glib::Propagation::Stop
                }
                (false, gtk::gdk::Key::Right) => {
                    (*goto_next)();
                    glib::Propagation::Stop
                }
                (false, gtk::gdk::Key::i) | (false, gtk::gdk::Key::I) => {
                    details_btn.set_active(!details_btn.is_active());
                    glib::Propagation::Stop
                }
                (false, gtk::gdk::Key::Escape) => {
                    ui.nav.pop();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        }
    ));
    page.add_controller(key_controller);

    // Ctrl+wheel zoom on the picture area, captured before the scrolled window
    // can use it for panning. Listening on both axes so trackpad two-finger
    // scrolls (which sometimes emit horizontal deltas) still trigger zoom.
    let zoom_scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    zoom_scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    zoom_scroll.connect_scroll(clone!(
        #[strong]
        zoom_by,
        move |ctrl, dx, dy| {
            let mods = ctrl.current_event_state();
            if !mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            let delta = if dy != 0.0 { dy } else { dx };
            if delta == 0.0 {
                return glib::Propagation::Proceed;
            }
            let factor = if delta < 0.0 { 1.1 } else { 1.0 / 1.1 };
            (*zoom_by)(factor);
            glib::Propagation::Stop
        }
    ));
    scrolled_picture.add_controller(zoom_scroll);

    download.connect_clicked(clone!(
        #[strong]
        ui,
        #[strong]
        pos_cell,
        move |_| {
            let pos = pos_cell.get();
            if let Some(item) = ui.grid.model.item(pos).and_downcast::<AssetObject>() {
                let asset_id = item.property::<String>("id");
                let filename = item.property::<String>("filename");
                if !asset_id.starts_with(LOCAL_ID_PREFIX) {
                    start_download(ui.clone(), asset_id, filename);
                }
            }
        }
    ));

    resolution_toggle.connect_toggled(clone!(
        #[strong]
        render,
        move |btn| {
            btn.set_label(if btn.is_active() {
                "Original"
            } else {
                "Preview"
            });
            (*render)();
        }
    ));

    ui.nav.push(&page);
}

/// Hand a local file off to the user's default app via `xdg-open`/equivalent.
/// Used for local videos per the spec — no in-app playback in v1.
fn open_local_with_default_app(path: &str) {
    let uri = format!("file://{}", path);
    if let Err(err) =
        gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>)
    {
        log::warn!("Failed to open {}: {}", uri, err);
    }
}

fn spawn_video_handoff(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String) {
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

fn start_download(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String) {
    begin_download_session(&ui.ctx, filename.clone());
    start_download_with_session(ui, asset_id, filename, true);
}

fn start_download_group(ui: Rc<LibraryWindowUi>, downloads: Vec<(String, String)>) {
    begin_download_session(&ui.ctx, format!("{} items", downloads.len()));
    for (asset_id, filename) in downloads {
        start_download_with_session(ui.clone(), asset_id, filename, false);
    }
}

fn start_download_with_session(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    filename: String,
    show_result_dialog: bool,
) {
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
                        #[strong]
                        show_result_dialog,
                        move |dialog, response| {
                            dialog.close();
                            if response == "overwrite" {
                                spawn_download(
                                    ui.clone(),
                                    asset_id.clone(),
                                    target_dir.join(&filename),
                                    show_result_dialog,
                                );
                            }
                        }
                    ),
                );
                dialog.present(Some(&ui.window));
                return;
            }
            spawn_download(ui, asset_id, output_path, show_result_dialog);
        }
    ));
}

fn spawn_download(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    output_path: PathBuf,
    show_result_dialog: bool,
) {
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let item_label = output_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| output_path.display().to_string());
            let progress =
                track_download_item(&ui.ctx, asset_id.clone(), Some(item_label.clone()), None);
            match ui
                .ctx
                .api_client
                .download_original_to_file(&asset_id, &output_path, Some(progress))
                .await
            {
                Ok(()) => {
                    let session_finished = {
                        finish_download_item(&ui.ctx, &asset_id);
                        !ui.ctx.state.lock().transfer.active
                    };
                    if should_refresh_after_download(&ui) && session_finished {
                        refresh_library_after_mutation(ui.clone(), true);
                    }
                    if show_result_dialog {
                        let heading = "Download Complete";
                        let body = format!("Saved {}", output_path.display());
                        let alert = libadwaita::AlertDialog::builder()
                            .heading(heading)
                            .body(&body)
                            .build();
                        alert.add_response("ok", "OK");
                        alert.present(Some(&ui.window));
                    }
                }
                Err(err) => {
                    finish_download_item(&ui.ctx, &asset_id);
                    if show_result_dialog {
                        let alert = libadwaita::AlertDialog::builder()
                            .heading("Download Failed")
                            .body(&err)
                            .build();
                        alert.add_response("ok", "OK");
                        alert.present(Some(&ui.window));
                    }
                }
            }
        }
    ));
}

async fn ensure_download_target(ui: &LibraryWindowUi) -> Option<PathBuf> {
    let existing_target = ui.ctx.config.read().data.download_target_path.clone();
    if let Some(path) = existing_target {
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
    {
        let mut config = ui.ctx.config.write();
        config.data.download_target_path = Some(path.to_string_lossy().to_string());
        let _ = config.save();
    }
    Some(path)
}

fn local_to_library_asset(local: LocalAsset) -> LibraryAsset {
    LibraryAsset {
        id: format!("{}{}", LOCAL_ID_PREFIX, local.path.display()),
        filename: local.filename,
        mime_type: local.mime,
        created_at: local.created_at,
        asset_type: local.asset_type.to_string(),
        thumbhash: None,
        width: None,
        height: None,
        checksum: None,
    }
}

async fn merge_unified_page(
    remote: Result<(Vec<LibraryAsset>, bool), String>,
    page: u32,
    ui: &Rc<LibraryWindowUi>,
    query: Option<&str>,
) -> Result<(Vec<LibraryAsset>, bool), String> {
    let (mut remote, has_more) = remote?;
    if page > 1 {
        return Ok((remote, has_more));
    }

    let mut locals = enumerate_local(ui.ctx.clone()).await;
    if let Some(q) = query {
        locals = filter_by_filename(locals, q);
    }

    let synced_paths: std::collections::HashSet<String> = {
        remote
            .iter()
            .filter_map(|a| a.checksum.as_deref())
            .filter_map(immich_checksum_to_hex)
            .filter_map(|hex| ui.ctx.sync_index.local_path_for_checksum(&hex))
            .collect()
    };

    let mut local_rows: Vec<LibraryAsset> = locals
        .into_iter()
        .filter(|l| !synced_paths.contains(&l.path.display().to_string()))
        .map(local_to_library_asset)
        .collect();

    local_rows.append(&mut remote);
    Ok((local_rows, has_more))
}

/// Return the path of the watch entry linked to `album_name`, if any.
fn linked_entry_path_for_album(ui: &Rc<LibraryWindowUi>, album_name: &str) -> Option<String> {
    let entries = ui.ctx.live_watch_paths.lock().clone();
    crate::config::watch_entry_for_album(album_name, &entries).map(|e| e.path().to_string())
}

/// Album-scoped variant of `merge_unified_page`: takes the album's asset
/// page from the remote API and overlays sync state from the album's
/// linked local folder only — never from siblings.
async fn merge_album_unified_page(
    remote: Result<(Vec<LibraryAsset>, bool), String>,
    page: u32,
    ui: &Rc<LibraryWindowUi>,
    album_name: &str,
) -> Result<(Vec<LibraryAsset>, bool), String> {
    let (mut remote, has_more) = remote?;
    if page > 1 {
        return Ok((remote, has_more));
    }

    let locals = match linked_entry_path_for_album(ui, album_name) {
        Some(path) => enumerate_local_for_entry(ui.ctx.clone(), path).await,
        None => Vec::new(),
    };

    let synced_paths: std::collections::HashSet<String> = {
        remote
            .iter()
            .filter_map(|a| a.checksum.as_deref())
            .filter_map(immich_checksum_to_hex)
            .filter_map(|hex| ui.ctx.sync_index.local_path_for_checksum(&hex))
            .collect()
    };

    let mut local_rows: Vec<LibraryAsset> = locals
        .into_iter()
        .filter(|l| !synced_paths.contains(&l.path.display().to_string()))
        .map(local_to_library_asset)
        .collect();

    local_rows.append(&mut remote);
    Ok((local_rows, has_more))
}

fn connect_filters_button(ui: Rc<LibraryWindowUi>, filters_button: gtk::Button) {
    filters_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            present_advanced_filters_dialog(ui.clone());
        }
    ));
}

fn present_advanced_filters_dialog(ui: Rc<LibraryWindowUi>) {
    let dialog = libadwaita::Dialog::builder()
        .title("Advanced Filters")
        .content_width(520)
        .content_height(720)
        .build();
    let toolbar = libadwaita::ToolbarView::builder().build();
    let header = libadwaita::HeaderBar::builder().build();
    toolbar.add_top_bar(&header);

    let page = libadwaita::PreferencesPage::new();

    let text_group = libadwaita::PreferencesGroup::builder()
        .title("Text")
        .description(
            "Description = user-set caption. OCR = text recognised inside images by Immich's ML \
             pipeline. All three are independent filter dimensions on /api/search/metadata.",
        )
        .build();
    let filename_row = libadwaita::EntryRow::builder()
        .title("Filename contains")
        .build();
    let description_row = libadwaita::EntryRow::builder()
        .title("Description contains")
        .build();
    let ocr_row = libadwaita::EntryRow::builder()
        .title("OCR text in image contains")
        .build();
    text_group.add(&filename_row);
    text_group.add(&description_row);
    text_group.add(&ocr_row);
    page.add(&text_group);

    // --- Type & flags ---
    let flags_group = libadwaita::PreferencesGroup::builder()
        .title("Type and flags")
        .build();
    let type_model = gtk::StringList::new(&["Any", "Image only", "Video only"]);
    let type_row = libadwaita::ComboRow::builder()
        .title("Asset type")
        .model(&type_model)
        .build();
    let favorite_row = libadwaita::SwitchRow::builder()
        .title("Favourites only")
        .build();
    let archived_row = libadwaita::SwitchRow::builder()
        .title("Archived only")
        .build();
    let motion_row = libadwaita::SwitchRow::builder()
        .title("Motion photos only")
        .build();
    let not_in_album_row = libadwaita::SwitchRow::builder()
        .title("Not in any album")
        .build();
    flags_group.add(&type_row);
    flags_group.add(&favorite_row);
    flags_group.add(&archived_row);
    flags_group.add(&motion_row);
    flags_group.add(&not_in_album_row);
    page.add(&flags_group);

    // --- Date range ---
    let date_group = libadwaita::PreferencesGroup::builder()
        .title("Date range")
        .description("ISO 8601 timestamps, e.g. 2024-01-15 or 2024-01-15T00:00:00Z")
        .build();
    let after_row = libadwaita::EntryRow::builder().title("Taken after").build();
    let before_row = libadwaita::EntryRow::builder()
        .title("Taken before")
        .build();
    date_group.add(&after_row);
    date_group.add(&before_row);
    page.add(&date_group);

    // --- Camera ---
    let camera_group = libadwaita::PreferencesGroup::builder()
        .title("Camera")
        .build();
    let make_row = libadwaita::EntryRow::builder().title("Make").build();
    let model_row = libadwaita::EntryRow::builder().title("Model").build();
    let lens_row = libadwaita::EntryRow::builder().title("Lens model").build();
    camera_group.add(&make_row);
    camera_group.add(&model_row);
    camera_group.add(&lens_row);
    page.add(&camera_group);

    // --- Location ---
    let loc_group = libadwaita::PreferencesGroup::builder()
        .title("Location")
        .build();
    let country_row = libadwaita::EntryRow::builder().title("Country").build();
    let state_row = libadwaita::EntryRow::builder()
        .title("State / region")
        .build();
    let city_row = libadwaita::EntryRow::builder().title("City").build();
    loc_group.add(&country_row);
    loc_group.add(&state_row);
    loc_group.add(&city_row);
    page.add(&loc_group);

    // --- Action buttons ---
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(12)
        .margin_bottom(12)
        .margin_end(12)
        .build();
    let cancel_btn = gtk::Button::builder().label("Cancel").build();
    let apply_btn = gtk::Button::builder()
        .label("Apply")
        .css_classes(vec!["suggested-action".to_string()])
        .build();
    actions.append(&cancel_btn);
    actions.append(&apply_btn);

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    outer.append(&page);
    outer.append(&actions);
    toolbar.set_content(Some(&outer));
    dialog.set_child(Some(&toolbar));

    cancel_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    apply_btn.connect_clicked(clone!(
        #[strong]
        ui,
        #[weak]
        dialog,
        #[weak]
        filename_row,
        #[weak]
        description_row,
        #[weak]
        ocr_row,
        #[weak]
        type_row,
        #[weak]
        favorite_row,
        #[weak]
        archived_row,
        #[weak]
        motion_row,
        #[weak]
        not_in_album_row,
        #[weak]
        after_row,
        #[weak]
        before_row,
        #[weak]
        make_row,
        #[weak]
        model_row,
        #[weak]
        lens_row,
        #[weak]
        country_row,
        #[weak]
        state_row,
        #[weak]
        city_row,
        move |_| {
            let filters = MetadataSearchFilters {
                original_file_name: opt_string(&filename_row.text()),
                description: opt_string(&description_row.text()),
                ocr: opt_string(&ocr_row.text()),
                asset_type: match type_row.selected() {
                    1 => Some("IMAGE".into()),
                    2 => Some("VIDEO".into()),
                    _ => None,
                },
                taken_after: normalise_iso_date(&after_row.text()),
                taken_before: normalise_iso_date(&before_row.text()),
                make: opt_string(&make_row.text()),
                model: opt_string(&model_row.text()),
                lens_model: opt_string(&lens_row.text()),
                country: opt_string(&country_row.text()),
                state: opt_string(&state_row.text()),
                city: opt_string(&city_row.text()),
                is_favorite: opt_true(favorite_row.is_active()),
                is_archived: opt_true(archived_row.is_active()),
                is_motion: opt_true(motion_row.is_active()),
                is_not_in_album: opt_true(not_in_album_row.is_active()),
                with_exif: None,
                with_deleted: None,
                person_ids: None,
                tag_ids: None,
                order: None,
            };
            let request =
                ui.ctx
                    .library_state
                    .lock()
                    .switch_source(LibrarySource::AdvancedSearch {
                        filters: Box::new(filters),
                    });
            dialog.close();
            load_source_page(ui.clone(), request, false);
        }
    ));

    dialog.present(Some(&ui.window));
}

fn opt_string(text: &gtk::glib::GString) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn opt_true(active: bool) -> Option<bool> {
    if active { Some(true) } else { None }
}

fn normalise_iso_date(text: &gtk::glib::GString) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Already RFC3339? Pass through.
    if chrono::DateTime::parse_from_rfc3339(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }
    // Bare YYYY-MM-DD? Expand to midnight UTC.
    if chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d").is_ok() {
        return Some(format!("{}T00:00:00.000Z", trimmed));
    }
    None
}
