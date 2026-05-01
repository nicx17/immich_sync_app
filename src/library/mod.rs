//! Library view module -- browse, search, and download assets from an Immich server.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{LibraryAsset, MetadataSearchFilters, ThumbnailSize};
use crate::app_context::AppContext;
use crate::config::Config;
use crate::library::asset_object::AssetObject;
use crate::library::grid_view::{GridViewParts, build_grid_view, replace_model};
use crate::library::local_source::{
    LocalAsset, enumerate_local, filter_by_filename, local_sync_state,
};
use crate::library::sidebar::{SidebarParts, build_sidebar};
use crate::library::state::{LibraryLoadState, LibrarySortMode, LibrarySource};
use crate::settings_window::build_settings_window_with_parent;

const LOCAL_ID_PREFIX: &str = "local::";

pub mod asset_object;
pub mod grid_view;
pub mod local_source;
pub mod sidebar;
pub mod state;
pub mod style;
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
    status_dot: gtk::Label,
    search_entry: gtk::SearchEntry,
    search_mode: gtk::DropDown,
    sort_mode: gtk::DropDown,
    source_mode: gtk::DropDown,
    /// Sticky month/year heading shown above the grid in Timeline mode.
    /// Updated on scroll using the `created_at` of the topmost visible
    /// asset; hidden in non-timeline sources.
    timeline_banner: gtk::Label,
}

pub fn build_library_window(app: &libadwaita::Application, ctx: Arc<AppContext>) {
    style::ensure_registered();

    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Mimick Library")
        .default_width(1180)
        .default_height(780)
        .build();

    let header = libadwaita::HeaderBar::builder()
        .show_start_title_buttons(true)
        .show_end_title_buttons(true)
        .build();
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

    let source_mode_model = gtk::StringList::new(&["Remote", "Local", "Unified", "Timeline"]);
    let source_mode = gtk::DropDown::builder()
        .model(&source_mode_model)
        .selected(0)
        .tooltip_text("Asset source")
        .build();

    // Three distinct search dimensions, each routed to a different Immich
    // endpoint shape. Smart and OCR are *separate* fields on the Immich
    // search DTOs (`query` vs `ocr` per the live OpenAPI spec), so we
    // expose them independently rather than collapsing OCR into Smart.
    let search_mode_model =
        gtk::StringList::new(&["Filename", "Smart (CLIP context)", "OCR (text in images)"]);
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
    controls.append(&search_mode);
    controls.append(&search_entry);
    controls.append(&filters_button);
    controls.append(&sort_mode);

    // Timeline banner — shown only when the Timeline source is active. The
    // text updates on scroll to the month/year of the asset at the top of
    // the visible grid. Hidden by default to avoid eating vertical space
    // in other sources.
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
    // `footer_label` carries the server stats + version. Pushing it to the
    // far right with hexpand keeps the spec's "status badge bottom-left"
    // layout while still surfacing the same data.
    let footer_label = gtk::Label::builder()
        .xalign(1.0)
        .hexpand(true)
        .wrap(true)
        .build();
    footer.append(&status_dot);
    footer.append(&route_label);
    footer.append(&footer_label);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&controls);
    content.append(&timeline_banner);
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
        status_dot,
        search_entry,
        search_mode,
        sort_mode,
        source_mode,
        timeline_banner,
    });

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
            // Pass the library window as parent so the settings open as a
            // transient/modal child, keeping the unified single-window feel
            // the spec calls for.
            build_settings_window_with_parent(&ui.app, ui.ctx.clone(), Some(&ui.window));
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

    ui.source_mode.connect_selected_notify(clone!(
        #[strong]
        ui,
        move |dropdown| {
            let source = match dropdown.selected() {
                1 => LibrarySource::LocalAll,
                2 => LibrarySource::Unified,
                3 => LibrarySource::Timeline,
                _ => LibrarySource::AllAssets,
            };
            // Searching while switching sources would require thread-safe
            // re-routing of the search field; clear it on source change.
            ui.search_entry.set_text("");
            // Timeline is meaningless without date-desc; force the sort
            // and reflect it in the dropdown so the user can see the
            // implicit override.
            if matches!(source, LibrarySource::Timeline) {
                ui.sort_mode.set_selected(0);
            }
            ui.timeline_banner
                .set_visible(matches!(source, LibrarySource::Timeline));
            let request = ui.ctx.library_state.lock().unwrap().switch_source(source);
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

            // Source dropdown picks the search dimension: Local/Unified use
            // filename filtering; Remote uses Filename/Smart/OCR as selected
            // in the search-mode dropdown. Indices line up with
            // `search_mode_model`: 0 = Filename, 1 = Smart, 2 = OCR.
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
            load_source_page(ui.clone(), request, false);
        }
    ));

    // Placeholder mirrors the selected search mode so the user sees the
    // semantic shift the moment they change the dropdown — avoids the
    // "why is my filename query returning weird CLIP matches" surprise.
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

            // `reload_sidebar` clears and re-selects rows programmatically, which makes GTK
            // re-emit `row-selected` for the same source. Skip when nothing actually changed
            // so we don't kick off a redundant fetch that loops via reload_sidebar forever.
            if ui.ctx.library_state.lock().unwrap().source == source {
                return;
            }

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

            open_lightbox(ui.clone(), asset_id, filename, local_path);
        }
    ));

    ui.grid.scrolled.vadjustment().connect_value_changed(clone!(
        #[strong]
        ui,
        move |adj| {
            // Timeline banner — update on every scroll tick when the
            // Timeline source is active. We approximate the topmost
            // visible row by mapping scroll fraction → asset index, then
            // reading `created_at` off that row's `LibraryAsset`.
            update_timeline_banner_if_active(&ui, adj);

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

fn update_timeline_banner_if_active(ui: &Rc<LibraryWindowUi>, adj: &gtk::Adjustment) {
    let state = ui.ctx.library_state.lock().unwrap();
    if !matches!(state.source, LibrarySource::Timeline) {
        return;
    }
    if state.assets.is_empty() {
        ui.timeline_banner.set_label("");
        return;
    }
    // Approximate "row at top of viewport" by mapping the scroll fraction
    // onto the asset index. Exact row geometry would require querying the
    // GridView, which doesn't expose its layout directly.
    let max = (adj.upper() - adj.page_size()).max(1.0);
    let frac = (adj.value() / max).clamp(0.0, 1.0);
    let idx = ((state.assets.len() as f64) * frac) as usize;
    let idx = idx.min(state.assets.len() - 1);
    let label = month_year_label(&state.assets[idx].created_at);
    ui.timeline_banner.set_label(&label);
}

/// Extract a "Month YYYY" heading from an ISO-8601 timestamp. Falls back to
/// the raw prefix if parsing fails so the banner is never blank for a
/// well-formed but unexpected format.
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

fn load_source_page(ui: Rc<LibraryWindowUi>, request: (u64, LibrarySource, u32), append: bool) {
    ui.content_stack.set_visible_child_name("loading");
    glib::MainContext::default().spawn_local(clone!(
        #[strong]
        ui,
        async move {
            let (generation, source, page) = request;
            let result: Result<Vec<LibraryAsset>, String> = match source.clone() {
                LibrarySource::AllAssets | LibrarySource::Timeline => {
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
            // Match on the album-id prefix delimited by ':' so that one album id being a
            // textual prefix of another never causes a false selection.
            let mut child = ui.sidebar.list.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if let Ok(row) = widget.downcast::<gtk::ListBoxRow>()
                    && row.tooltip_text().as_deref().is_some_and(|tooltip| {
                        tooltip.split_once(':').map(|(prefix, _)| prefix) == Some(id.as_str())
                    })
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

fn open_lightbox(ui: Rc<LibraryWindowUi>, asset_id: String, filename: String, local_path: String) {
    // `adw::Dialog` floats over the library window instead of opening a new
    // top-level. Adwaita handles Escape-to-close and focus restoration; we
    // only have to wire the actual content. The header bar is empty
    // because the dialog provides its own close affordance.
    let dialog = libadwaita::Dialog::builder()
        .title(&filename)
        .content_width(980)
        .content_height(760)
        .build();
    let toolbar = libadwaita::ToolbarView::builder().build();
    let header = libadwaita::HeaderBar::builder().build();
    toolbar.add_top_bar(&header);
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

    // Per-image preview/original toggle. Default mirrors the Config setting,
    // and flipping it triggers a re-fetch via the same loader path.
    let initial_full = Config::new().data.library_preview_full_resolution;
    let resolution_toggle = gtk::ToggleButton::builder()
        .label(if initial_full { "Original" } else { "Preview" })
        .tooltip_text("Toggle preview vs original full-resolution image")
        .active(initial_full)
        .build();

    let download = gtk::Button::builder().label("Download").build();
    let close_btn = gtk::Button::builder().label("Close").build();

    // For purely local rows, the network-side toggle is meaningless (the
    // file IS the original on disk), so hide both.
    let is_local = !local_path.is_empty() && asset_id.starts_with(LOCAL_ID_PREFIX);
    if is_local {
        resolution_toggle.set_visible(false);
        download.set_visible(false);
    }
    actions.append(&resolution_toggle);
    actions.append(&download);
    actions.append(&close_btn);
    content.append(&picture);
    content.append(&actions);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    close_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    if !is_local {
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
    }

    let load_into_picture = {
        let ui = ui.clone();
        let asset_id = asset_id.clone();
        let local_path = local_path.clone();
        let picture = picture.clone();
        std::rc::Rc::new(move |full_res: bool| {
            let ui = ui.clone();
            let asset_id = asset_id.clone();
            let local_path = local_path.clone();
            let picture = picture.clone();
            glib::MainContext::default().spawn_local(async move {
                if !local_path.is_empty() {
                    if let Ok(texture) = gdk4::Texture::from_filename(&local_path) {
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
                        if let Ok(texture) = gdk4::Texture::from_filename(&temp) {
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
        })
    };

    resolution_toggle.connect_toggled(clone!(
        #[strong]
        load_into_picture,
        move |btn| {
            let full = btn.is_active();
            btn.set_label(if full { "Original" } else { "Preview" });
            (*load_into_picture)(full);
        }
    ));

    (*load_into_picture)(initial_full);

    dialog.present(Some(&ui.window));
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

fn connect_filters_button(ui: Rc<LibraryWindowUi>, filters_button: gtk::Button) {
    filters_button.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            present_advanced_filters_dialog(ui.clone());
        }
    ));
}

/// Build and present the advanced-filters dialog. The dialog wraps an
/// `adw::PreferencesPage` so each filter dimension renders as an
/// `AdwActionRow` / `AdwSwitchRow` / `AdwEntryRow` — the same design
/// language as the main settings window — and submits a populated
/// `MetadataSearchFilters` via the `LibrarySource::AdvancedSearch` path.
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
