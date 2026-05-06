//! Library view module -- browse, search, and download assets from an Immich server.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{LibraryAsset, MetadataSearchFilters, ThumbnailSize};
use crate::app_context::AppContext;
use crate::config::Config;
use crate::library::albums_view::{
    AlbumClick, AlbumsViewParts, build_albums_view, populate_albums,
};
use crate::library::asset_object::AssetObject;
use crate::library::explore_view::{ExploreViewParts, build_explore_view};
use crate::library::grid_view::{GridViewParts, build_grid_view, extend_model, replace_model};
use crate::library::local_source::{
    LocalAsset, enumerate_local, enumerate_local_for_entry, filter_by_filename, local_sync_state,
};
use crate::library::sidebar::{SidebarParts, build_sidebar};
use crate::library::state::{LibraryLoadState, LibrarySortMode, LibrarySource};
use crate::settings_window::build_settings_window_with_parent;

const LOCAL_ID_PREFIX: &str = "local::";

pub mod album_sync;
pub mod albums_view;
pub mod asset_object;
pub mod explore_view;
pub mod grid_view;
pub mod local_source;
pub mod sidebar;
pub mod state;
pub mod style;
pub mod thumbnail_cache;

const PAGE_SIZE: u32 = 50;

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
    footer_label: gtk::Label,
    route_label: gtk::Label,
    status_dot: gtk::Label,
    search_entry: gtk::SearchEntry,
    search_mode: gtk::DropDown,
    sort_mode: gtk::DropDown,
    source_mode: gtk::DropDown,
    filters_button: gtk::Button,
    timeline_toggle: gtk::ToggleButton,
    timeline_banner: gtk::Label,
    source_mode_suppressed: Cell<bool>,
    sidebar_suppressed: Cell<bool>,
    select_toggle: gtk::ToggleButton,
    bulk_bar: gtk::Revealer,
    bulk_count_label: gtk::Label,
    album_link_row: libadwaita::ActionRow,
    album_link_button: gtk::Button,
    album_sync_button: gtk::Button,
}

pub fn build_library_window(app: &libadwaita::Application, ctx: Arc<AppContext>) {
    style::ensure_registered();

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
    let prefs_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Open Settings")
        .build();
    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .build();
    header.pack_start(&sidebar_toggle);
    header.pack_end(&prefs_button);
    header.pack_end(&refresh_button);
    let select_toggle = gtk::ToggleButton::builder()
        .icon_name("checkbox-symbolic")
        .tooltip_text("Select assets (Esc to exit)")
        .build();
    header.pack_end(&select_toggle);

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

    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let status_dot = gtk::Label::builder()
        .label("\u{25CF}")
        .css_classes(vec!["mimick-status-dot".to_string(), "offline".to_string()])
        .build();
    let route_label = gtk::Label::builder().xalign(0.0).build();

    let footer_label = gtk::Label::builder()
        .xalign(1.0)
        .hexpand(true)
        .wrap(true)
        .build();
    footer.append(&status_dot);
    footer.append(&route_label);
    footer.append(&footer_label);

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
    content.append(&footer);

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
        footer_label,
        route_label,
        status_dot,
        search_entry,
        search_mode,
        sort_mode,
        source_mode,
        filters_button: filters_button.clone(),
        timeline_toggle,
        timeline_banner,
        source_mode_suppressed: Cell::new(false),
        sidebar_suppressed: Cell::new(false),
        select_toggle: select_toggle.clone(),
        bulk_bar: bulk_bar.clone(),
        bulk_count_label: bulk_count_label.clone(),
        album_link_row: album_link_row.clone(),
        album_link_button: album_link_button.clone(),
        album_sync_button: album_sync_button.clone(),
    });

    connect_album_link_row(ui.clone(), album_link_listbox);

    connect_select_mode(ui.clone(), select_toggle.clone());
    connect_bulk_actions(ui.clone(), bulk_delete, bulk_download, bulk_clear);

    connect_sidebar_handlers(ui.clone());
    connect_controls(ui.clone(), prefs_button, refresh_button);
    connect_grid_handlers(ui.clone());
    connect_filters_button(ui.clone(), filters_button);

    bootstrap_window(ui);
    window.present();
}

fn bootstrap_window(ui: Rc<LibraryWindowUi>) {
    let initial_request = {
        let mut state = ui.ctx.library_state.lock().unwrap();
        state.load_initial_source()
    };

    apply_timeline_ui_state(&ui, &initial_request.1);
    load_albums(ui.clone());
    load_status(ui.clone());
    fetch_current_user(ui.clone());
    connect_albums_create(ui.clone());
    spawn_server_ping_loop(ui.clone());
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

fn fetch_current_user(ui: Rc<LibraryWindowUi>) {
    if ui.ctx.current_user_id.lock().unwrap().is_some() {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_current_user_id().await {
            Ok(id) => {
                *ui.ctx.current_user_id.lock().unwrap() = Some(id);
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
                    refresh_albums_view(ui.clone());
                    reload_sidebar(&ui);
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
    refresh_button: gtk::Button,
) {
    prefs_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            build_settings_window_with_parent(&ui.app, ui.ctx.clone(), Some(&ui.window));
        }
    ));

    refresh_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            load_albums(ui.clone());
            load_status(ui.clone());
            ui.explore.populated.set(false);
            let request = {
                let source = ui.ctx.library_state.lock().unwrap().source.clone();
                ui.ctx.library_state.lock().unwrap().switch_source(source)
            };
            load_source_page(ui.clone(), request, false);
        }
    ));

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
            let album_ctx = match ui.ctx.library_state.lock().unwrap().source.clone() {
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
            let request = ui.ctx.library_state.lock().unwrap().switch_source(source);
            apply_timeline_ui_state(&ui, &request.1);
            load_source_page(ui.clone(), request, false);
        }
    ));

    ui.timeline_toggle.connect_toggled(clone!(
        #[strong]
        ui,
        move |toggle| {
            if !toggle.is_sensitive() {
                return;
            }
            let current = ui.ctx.library_state.lock().unwrap().source.clone();
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
            let request = ui
                .ctx
                .library_state
                .lock()
                .unwrap()
                .switch_source(next_source);
            apply_timeline_ui_state(&ui, &request.1);
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

            let source = match ui.source_mode.selected() {
                1 => LibrarySource::LocalSearch { query },
                2 => LibrarySource::UnifiedSearch { query },
                _ => match ui.search_mode.selected() {
                    1 => LibrarySource::SmartSearch { query },
                    2 => LibrarySource::OcrSearch { query },
                    _ => LibrarySource::MetadataSearch { query },
                },
            };
            let request = ui.ctx.library_state.lock().unwrap().switch_source(source);
            apply_timeline_ui_state(&ui, &request.1);
            load_source_page(ui.clone(), request, false);
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
                .unwrap()
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
    key_controller.connect_key_pressed({
        let select_toggle = select_toggle.clone();
        let transient = transient.clone();
        move |_, keyval, _, _| {
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
        }
    ));

    download_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            for (asset_id, filename) in collect_selected_assets(&ui) {
                if !asset_id.starts_with(LOCAL_ID_PREFIX) {
                    start_download(ui.clone(), asset_id, filename);
                }
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
                    reload_sidebar(&ui);
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

/// Common path for sidebar selections. Skips redundant dispatches so the
/// `reload_sidebar` programmatic re-selection doesn't loop into another
/// fetch — but only when the content stack is already showing the current
/// source, since the Albums grid moves the stack without changing source.
fn sidebar_dispatch(ui: Rc<LibraryWindowUi>, source: LibrarySource) {
    let on_albums_grid = ui.content_stack.visible_child_name().as_deref() == Some("albums");
    if !on_albums_grid && ui.ctx.library_state.lock().unwrap().source == source {
        return;
    }
    let request = ui.ctx.library_state.lock().unwrap().switch_source(source);
    apply_timeline_ui_state(&ui, &request.1);
    load_source_page(ui.clone(), request, false);
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
            });
        }
    ));
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

    let entries = ui.ctx.live_watch_paths.lock().unwrap().clone();
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
    let source = ui.ctx.library_state.lock().unwrap().source.clone();
    let LibrarySource::Album {
        id: album_id,
        name: album_name,
    } = source
    else {
        return;
    };
    let entries = ui.ctx.live_watch_paths.lock().unwrap().clone();
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
        });
        dlg.close();
    });
    dialog.present(Some(&ui.window));
}

fn handle_album_link_click(ui: Rc<LibraryWindowUi>) {
    let source = ui.ctx.library_state.lock().unwrap().source.clone();
    let LibrarySource::Album {
        id: album_id,
        name: album_name,
    } = source
    else {
        return;
    };

    let entries = ui.ctx.live_watch_paths.lock().unwrap().clone();
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
    let mut config = Config::new();
    config
        .data
        .watch_paths
        .retain(|entry| entry.album_name() != Some(album_name));
    if !config.save() {
        log::error!("Failed to save config after unlink");
        return;
    }
    *ui.ctx.live_watch_paths.lock().unwrap() = config.data.watch_paths.clone();
    let source_after = ui.ctx.library_state.lock().unwrap().source.clone();
    refresh_album_link_row(&ui, &source_after);
}

fn link_album_to_path(
    ui: Rc<LibraryWindowUi>,
    album_id: String,
    album_name: String,
    path: std::path::PathBuf,
) {
    let path_string = path.to_string_lossy().to_string();
    let mut config = Config::new();
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
    *ui.ctx.live_watch_paths.lock().unwrap() = config.data.watch_paths.clone();
    let source_after = ui.ctx.library_state.lock().unwrap().source.clone();
    refresh_album_link_row(&ui, &source_after);
}

fn update_timeline_banner_if_active(ui: &Rc<LibraryWindowUi>, adj: &gtk::Adjustment) {
    let state = ui.ctx.library_state.lock().unwrap();
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

            let click_ui = ui.clone();
            explore_view::populate_people(&ui.explore, ctx.clone(), people, move |id, name| {
                let filters = MetadataSearchFilters {
                    person_ids: Some(vec![id]),
                    ..Default::default()
                };
                let request = click_ui.ctx.library_state.lock().unwrap().switch_source(
                    LibrarySource::AdvancedSearch {
                        filters: Box::new(filters),
                    },
                );
                click_ui.search_entry.set_text(&name);
                apply_timeline_ui_state(&click_ui, &request.1);
                load_source_page(click_ui.clone(), request, false);
            });
            let click_ui = ui.clone();
            explore_view::populate_explore(
                &ui.explore,
                ctx.clone(),
                sections,
                move |kind, value| {
                    let next = match kind {
                        "place" => LibrarySource::AdvancedSearch {
                            filters: Box::new(MetadataSearchFilters {
                                city: Some(value.clone()),
                                ..Default::default()
                            }),
                        },
                        _ => LibrarySource::SmartSearch {
                            query: value.clone(),
                        },
                    };
                    let request = click_ui
                        .ctx
                        .library_state
                        .lock()
                        .unwrap()
                        .switch_source(next);
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
            let result: Result<Vec<LibraryAsset>, String> = match source.clone() {
                LibrarySource::AllAssets | LibrarySource::Timeline => {
                    ui.ctx.api_client.search_metadata("", page, PAGE_SIZE).await
                }
                LibrarySource::Explore => unreachable!("intercepted above"),
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
                LibrarySource::OcrSearch { query } => {
                    ui.ctx.api_client.search_ocr(&query, page, PAGE_SIZE).await
                }
                LibrarySource::MetadataSearch { query } => {
                    ui.ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::AdvancedSearch { filters } => {
                    ui.ctx
                        .api_client
                        .search_metadata_with_filters(&filters, page, PAGE_SIZE)
                        .await
                }
                LibrarySource::LocalAll => {
                    // Local enumeration is bounded — single synthetic page.
                    if page > 1 {
                        Ok(Vec::new())
                    } else {
                        let locals = enumerate_local(ui.ctx.clone()).await;
                        Ok(locals.into_iter().map(local_to_library_asset).collect())
                    }
                }
                LibrarySource::LocalSearch { query } => {
                    if page > 1 {
                        Ok(Vec::new())
                    } else {
                        let locals = enumerate_local(ui.ctx.clone()).await;
                        let filtered = filter_by_filename(locals, &query);
                        Ok(filtered.into_iter().map(local_to_library_asset).collect())
                    }
                }
                LibrarySource::Unified => {
                    let remote = ui.ctx.api_client.search_metadata("", page, PAGE_SIZE).await;
                    merge_unified_page(remote, page, &ui, None).await
                }
                LibrarySource::UnifiedSearch { query } => {
                    let remote = ui
                        .ctx
                        .api_client
                        .search_metadata(&query, page, PAGE_SIZE)
                        .await;
                    merge_unified_page(remote, page, &ui, Some(&query)).await
                }
                LibrarySource::AlbumLocal { name, .. } => {
                    if page > 1 {
                        Ok(Vec::new())
                    } else {
                        match linked_entry_path_for_album(&ui, &name) {
                            Some(path) => {
                                let locals = enumerate_local_for_entry(ui.ctx.clone(), path).await;
                                Ok(locals.into_iter().map(local_to_library_asset).collect())
                            }
                            None => Ok(Vec::new()),
                        }
                    }
                }
                LibrarySource::AlbumUnified { id, name } => {
                    let remote = ui
                        .ctx
                        .api_client
                        .fetch_album_assets(&id, page, PAGE_SIZE)
                        .await;
                    merge_album_unified_page(remote, page, &ui, &name).await
                }
            };

            match result {
                Ok(items) => {
                    let outcome = {
                        let mut state = ui.ctx.library_state.lock().unwrap();
                        let prev_len = state.assets.len();
                        let applied = if append {
                            state.append_assets(generation, items)
                        } else {
                            state.replace_assets(generation, items)
                        };
                        if !applied {
                            return;
                        }
                        let objects = asset_objects_from_state(&state.assets, &ui.ctx);
                        (objects, prev_len)
                    };
                    let (objects, prev_len) = outcome;
                    if append && prev_len <= objects.len() {
                        extend_model(&ui.grid.model, &objects[prev_len..]);
                    } else {
                        replace_model(&ui.grid.model, &objects);
                    }
                    sync_content_state(&ui);
                    reload_sidebar(&ui);
                    update_timeline_banner_if_active(&ui, &ui.grid.scrolled.vadjustment());
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

fn reload_sidebar(ui: &Rc<LibraryWindowUi>) {
    while let Some(row) = ui.sidebar.albums_list.first_child() {
        ui.sidebar.albums_list.remove(&row);
    }

    let selected_source = ui.ctx.library_state.lock().unwrap().source.clone();
    let albums = ui.ctx.library_state.lock().unwrap().albums.clone();
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
        ui.status_dot.remove_css_class("offline");
        ui.status_dot.add_css_class("connected");
    } else {
        ui.route_label.set_label("Offline");
        ui.status_dot.remove_css_class("connected");
        ui.status_dot.add_css_class("offline");
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

fn asset_objects_from_state(assets: &[LibraryAsset], ctx: &AppContext) -> Vec<AssetObject> {
    let sync_index = ctx.sync_index.lock().unwrap();
    assets
        .iter()
        .map(|asset| {
            if let Some(local_path) = asset.id.strip_prefix(LOCAL_ID_PREFIX) {
                let sync_state = local_sync_state(&sync_index, std::path::Path::new(local_path));
                let object = AssetObject::new_local(
                    &asset.id,
                    &asset.filename,
                    &asset.mime_type,
                    &asset.created_at,
                    &asset.asset_type,
                    local_path,
                );
                if sync_state != 1 {
                    object.set_property("sync-state", sync_state);
                }
                return object;
            }
            // Remote rows: 2 = "both" when a sibling local copy exists, else 0 (remote-only).
            let local_match = asset
                .checksum
                .as_deref()
                .and_then(|checksum| sync_index.local_path_for_checksum(checksum));
            let sync_state = if local_match.is_some() { 2 } else { 0 };
            let object = AssetObject::new(
                &asset.id,
                &asset.filename,
                &asset.mime_type,
                &asset.created_at,
                &asset.asset_type,
                sync_state,
                asset.thumbhash.as_deref(),
            );
            if let Some(path) = local_match {
                object.set_property("local-path", path);
            }
            object
        })
        .collect()
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
        rows.push(("Taken".into(), dt.clone()));
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
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .vexpand(true)
        .hexpand(true)
        .build();
    let initial_full = Config::new().data.library_preview_full_resolution;
    let resolution_toggle = gtk::ToggleButton::builder()
        .label(if initial_full { "Original" } else { "Preview" })
        .tooltip_text("Toggle preview vs original full-resolution image")
        .active(initial_full)
        .build();
    let download = gtk::Button::builder().label("Download").build();
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&resolution_toggle);
    actions.append(&download);
    viewer.append(&picture);
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
        .width_request(320)
        .visible(false)
        .build();
    details_pane.set_size_request(320, -1);
    details_pane.set_propagate_natural_width(false);
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
        let picture = picture.clone();
        move |asset_id: String, local_path: String, full_res: bool| {
            let ui = ui.clone();
            let picture = picture.clone();
            glib::MainContext::default().spawn_local(async move {
                if !local_path.is_empty() {
                    if let Some(texture) = load_texture_oriented(std::path::Path::new(&local_path))
                    {
                        picture.set_paintable(Some(&texture));
                    }
                    return;
                }
                if full_res {
                    if let Some(cache_dir) =
                        dirs::cache_dir().map(|p| p.join("mimick").join("preview"))
                    {
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let temp = cache_dir.join(format!("{}.bin", asset_id));
                        if !temp.exists()
                            && let Err(err) = ui
                                .ctx
                                .api_client
                                .download_original_to_file(&asset_id, &temp)
                                .await
                        {
                            log::warn!("Lightbox original fetch failed: {}", err);
                            return;
                        }
                        if let Some(texture) = load_texture_oriented(&temp) {
                            picture.set_paintable(Some(&texture));
                        }
                    }
                } else if let Ok(texture) = ui
                    .ctx
                    .thumbnail_cache
                    .load_thumbnail(&asset_id, ThumbnailSize::Preview)
                    .await
                {
                    picture.set_paintable(Some(&texture));
                }
            });
        }
    });

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
            details_summary.set_label(&format!("{} · {}\nCreated: {}", mime, sync_label, created));

            while let Some(c) = details_exif.first_child() {
                details_exif.remove(&c);
            }
            details_exif.set_visible(false);

            prev_btn.set_sensitive(pos > 0);
            next_btn.set_sensitive(pos + 1 < n);

            let is_local = !local_path.is_empty() && asset_id.starts_with(LOCAL_ID_PREFIX);
            resolution_toggle.set_visible(!is_local);
            download.set_visible(!is_local);

            (*load_into_picture)(asset_id.clone(), local_path, resolution_toggle.is_active());

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
        move |_| {
            let pos = pos_cell.get();
            if pos > 0 {
                pos_cell.set(pos - 1);
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
        move || {
            let pos = pos_cell.get();
            if pos + 1 < ui.grid.model.n_items() {
                pos_cell.set(pos + 1);
                (*render)();
                return;
            }
            let next_request = ui
                .ctx
                .library_state
                .lock()
                .unwrap()
                .load_next_page_if_needed();
            let Some(req) = next_request else {
                return;
            };
            next_btn.set_sensitive(false);
            let model = ui.grid.model.clone();
            let pos_cell_h = pos_cell.clone();
            let render_h = render.clone();
            let next_btn_h = next_btn.clone();
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
        move |_, key, _, _| match key {
            gtk::gdk::Key::Left => {
                let pos = pos_cell.get();
                if pos > 0 {
                    pos_cell.set(pos - 1);
                    (*render)();
                }
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Right => {
                (*goto_next)();
                glib::Propagation::Stop
            }
            gtk::gdk::Key::i | gtk::gdk::Key::I => {
                details_btn.set_active(!details_btn.is_active());
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Escape => {
                ui.nav.pop();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    ));
    page.add_controller(key_controller);

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
        let Some(cache_dir) = dirs::cache_dir().map(|p| p.join("mimick").join("video")) else {
            return;
        };
        let _ = std::fs::create_dir_all(&cache_dir);
        let path = cache_dir.join(&filename);
        if !path.exists()
            && let Err(err) = ui
                .ctx
                .api_client
                .download_original_to_file(&asset_id, &path)
                .await
        {
            log::warn!("Video handoff failed for {}: {}", asset_id, err);
            return;
        }
        open_local_with_default_app(&path.display().to_string());
    });
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
            match ui
                .ctx
                .api_client
                .download_original_to_file(&asset_id, &output_path)
                .await
            {
                Ok(()) => {
                    let heading = "Download Complete";
                    let body = format!("Saved {}", output_path.display());
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
    remote: Result<Vec<LibraryAsset>, String>,
    page: u32,
    ui: &Rc<LibraryWindowUi>,
    query: Option<&str>,
) -> Result<Vec<LibraryAsset>, String> {
    let mut remote = remote?;
    if page > 1 {
        return Ok(remote);
    }

    let mut locals = enumerate_local(ui.ctx.clone()).await;
    if let Some(q) = query {
        locals = filter_by_filename(locals, q);
    }

    let synced_paths: std::collections::HashSet<String> = match ui.ctx.sync_index.lock() {
        Ok(idx) => remote
            .iter()
            .filter_map(|a| a.checksum.as_deref())
            .filter_map(|cs| idx.local_path_for_checksum(cs))
            .collect(),
        Err(_) => std::collections::HashSet::new(),
    };

    let mut local_rows: Vec<LibraryAsset> = locals
        .into_iter()
        .filter(|l| !synced_paths.contains(&l.path.display().to_string()))
        .map(local_to_library_asset)
        .collect();

    local_rows.append(&mut remote);
    Ok(local_rows)
}

/// Return the path of the watch entry linked to `album_name`, if any.
fn linked_entry_path_for_album(ui: &Rc<LibraryWindowUi>, album_name: &str) -> Option<String> {
    let entries = ui.ctx.live_watch_paths.lock().ok()?.clone();
    crate::config::watch_entry_for_album(album_name, &entries).map(|e| e.path().to_string())
}

/// Album-scoped variant of `merge_unified_page`: takes the album's asset
/// page from the remote API and overlays sync state from the album's
/// linked local folder only — never from siblings.
async fn merge_album_unified_page(
    remote: Result<Vec<LibraryAsset>, String>,
    page: u32,
    ui: &Rc<LibraryWindowUi>,
    album_name: &str,
) -> Result<Vec<LibraryAsset>, String> {
    let mut remote = remote?;
    if page > 1 {
        return Ok(remote);
    }

    let locals = match linked_entry_path_for_album(ui, album_name) {
        Some(path) => enumerate_local_for_entry(ui.ctx.clone(), path).await,
        None => Vec::new(),
    };

    let synced_paths: std::collections::HashSet<String> = match ui.ctx.sync_index.lock() {
        Ok(idx) => remote
            .iter()
            .filter_map(|a| a.checksum.as_deref())
            .filter_map(|cs| idx.local_path_for_checksum(cs))
            .collect(),
        Err(_) => std::collections::HashSet::new(),
    };

    let mut local_rows: Vec<LibraryAsset> = locals
        .into_iter()
        .filter(|l| !synced_paths.contains(&l.path.display().to_string()))
        .map(local_to_library_asset)
        .collect();

    local_rows.append(&mut remote);
    Ok(local_rows)
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
            };
            let request =
                ui.ctx
                    .library_state
                    .lock()
                    .unwrap()
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
