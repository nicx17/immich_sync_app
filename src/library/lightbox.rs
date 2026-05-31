//! Lightbox image viewer: full-screen preview with zoom, pan, EXIF details, and keyboard navigation.
//!
//! Loads preview or original resolution images with pinch-zoom and
//! swipe navigation. Displays an EXIF metadata panel and provides
//! download-to-folder and delete-to-trash actions.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{ExifInfo, ThumbnailSize};
use crate::library::asset_object::AssetObject;
use crate::library::local_exif::{self, LocalExif};

use super::context_menu::show_asset_context_menu;
use super::download::{
    begin_download_session, finish_download_item, open_local_with_default_app, spawn_video_handoff,
    start_download, track_download_item,
};
use super::{LOCAL_ID_PREFIX, LibraryWindowUi, load_source_page, load_texture_oriented};

/// Everything we can learn about a local file off the main thread before
/// handing the data back to GTK. `exif` is the cached EXIF parse; `dims`
/// comes from a pixbuf header-only read; `mtime_iso` falls back when the
/// file has no `DateTimeOriginal` so we always show *some* date.
struct LocalProbe {
    exif: Option<LocalExif>,
    file_size: Option<u64>,
    mtime_iso: Option<String>,
    dims: Option<(u32, u32)>,
}

/// Render a `SystemTime` as an RFC3339 string in the local timezone so the
/// existing `format_datetime_display` pipeline can format it consistently
/// with EXIF-derived timestamps.
fn systemtime_to_rfc3339(t: std::time::SystemTime) -> String {
    use chrono::{DateTime, Local};
    let dt: DateTime<Local> = t.into();
    dt.to_rfc3339()
}

/// Project local-file metadata into the API EXIF shape so the existing
/// renderer handles both sources uniformly.
fn exif_from_local(local: &LocalExif, file_size: Option<u64>) -> ExifInfo {
    ExifInfo {
        make: local.make.clone(),
        model: local.model.clone(),
        lens_model: local.lens_model.clone(),
        f_number: local.f_number,
        focal_length: local.focal_length,
        iso: local.iso,
        exposure_time: local.exposure_time.clone(),
        file_size_in_byte: file_size,
        date_time_original: local.date_time_original.clone(),
        city: None,
        state: None,
        country: None,
        latitude: local.latitude,
        longitude: local.longitude,
        description: local.description.clone(),
        exif_image_width: local.image_width,
        exif_image_height: local.image_height,
    }
}

fn original_preview_cache_path(
    cache_dir: &std::path::Path,
    asset_id: &str,
    filename: &str,
) -> std::path::PathBuf {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .unwrap_or("bin");
    cache_dir.join(format!("{asset_id}.{ext}"))
}

/// Populate the details sidebar with sectioned EXIF metadata.
///
/// Groups fall into three categories — Camera, Image, Location — each rendered
/// as an `AdwPreferencesGroup` with accent-coloured prefix icons. Empty groups
/// (e.g. an image with no GPS) are skipped so the pane stays compact.
///
/// `taken_label` decides whether the date row reads "Taken" (EXIF
/// DateTimeOriginal — the camera's capture moment) or "Modified" (filesystem
/// mtime fallback when no capture timestamp exists).
fn fill_exif_box(container: &gtk::Box, exif: &crate::api_client::ExifInfo, taken_label: &str) {
    if let Some(group) = build_camera_group(exif) {
        container.append(&group);
    }
    if let Some(group) = build_image_group(exif, taken_label) {
        container.append(&group);
    }
    if let Some(group) = build_location_group(exif) {
        container.append(&group);
    }
    if let Some(desc) = &exif.description
        && !desc.trim().is_empty()
    {
        let note_group = libadwaita::PreferencesGroup::builder()
            .title("Description")
            .build();
        let row = libadwaita::ActionRow::builder()
            .title(desc.as_str())
            .title_lines(0)
            .css_classes(["property"])
            .build();
        note_group.add(&row);
        container.append(&note_group);
    }
}

fn build_camera_group(exif: &crate::api_client::ExifInfo) -> Option<libadwaita::PreferencesGroup> {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Camera")
        .build();
    let mut rows = 0u32;
    if let Some(c) = format_camera(exif) {
        group.add(&accent_row(
            "camera-photo-symbolic",
            "mimick-accent-camera",
            "Body",
            &c,
        ));
        rows += 1;
    }
    if let Some(l) = &exif.lens_model
        && !l.trim().is_empty()
    {
        group.add(&accent_row(
            "view-fullscreen-symbolic",
            "mimick-accent-camera",
            "Lens",
            l,
        ));
        rows += 1;
    }
    if let Some(exposure) = format_exposure(exif) {
        group.add(&accent_row(
            "weather-clear-symbolic",
            "mimick-accent-camera",
            "Exposure",
            &exposure,
        ));
        rows += 1;
    }
    (rows > 0).then_some(group)
}

fn build_image_group(
    exif: &crate::api_client::ExifInfo,
    taken_label: &str,
) -> Option<libadwaita::PreferencesGroup> {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Image")
        .build();
    let mut rows = 0u32;
    if let (Some(w), Some(h)) = (exif.exif_image_width, exif.exif_image_height) {
        group.add(&accent_row(
            "view-grid-symbolic",
            "mimick-accent-image",
            "Dimensions",
            &format!("{w} × {h}"),
        ));
        rows += 1;
    }
    if let Some(size) = exif.file_size_in_byte {
        group.add(&accent_row(
            "drive-harddisk-symbolic",
            "mimick-accent-image",
            "Size",
            &format_bytes(size),
        ));
        rows += 1;
    }
    if let Some(dt) = &exif.date_time_original
        && !dt.trim().is_empty()
    {
        group.add(&accent_row(
            "x-office-calendar-symbolic",
            "mimick-accent-image",
            taken_label,
            &format_datetime_display(dt),
        ));
        rows += 1;
    }
    (rows > 0).then_some(group)
}

fn build_location_group(
    exif: &crate::api_client::ExifInfo,
) -> Option<libadwaita::PreferencesGroup> {
    let group = libadwaita::PreferencesGroup::builder()
        .title("Location")
        .build();
    let mut rows = 0u32;
    if let Some(loc) = format_location(exif) {
        group.add(&accent_row(
            "mark-location-symbolic",
            "mimick-accent-location",
            "Place",
            &loc,
        ));
        rows += 1;
    }
    if let (Some(lat), Some(lon)) = (exif.latitude, exif.longitude) {
        group.add(&accent_row(
            "find-location-symbolic",
            "mimick-accent-location",
            "Coordinates",
            &format!("{lat:.5}, {lon:.5}"),
        ));
        rows += 1;
    }
    (rows > 0).then_some(group)
}

fn accent_row(icon: &str, accent_class: &str, title: &str, value: &str) -> libadwaita::ActionRow {
    let prefix = gtk::Image::builder()
        .icon_name(icon)
        .pixel_size(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .css_classes(["mimick-detail-icon", accent_class])
        .build();
    let row = libadwaita::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .subtitle_lines(2)
        .css_classes(["property"])
        .build();
    row.add_prefix(&prefix);
    row
}

fn format_camera(exif: &crate::api_client::ExifInfo) -> Option<String> {
    match (&exif.make, &exif.model) {
        (Some(m), Some(n)) => {
            let m = m.trim();
            let n = n.trim();
            if n.starts_with(m) {
                Some(n.to_string())
            } else {
                Some(format!("{m} {n}"))
            }
        }
        (Some(m), None) => Some(m.trim().to_string()),
        (None, Some(n)) => Some(n.trim().to_string()),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

fn format_exposure(exif: &crate::api_client::ExifInfo) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(f) = exif.f_number {
        parts.push(format!("ƒ/{f:.1}"));
    }
    if let Some(et) = &exif.exposure_time
        && !et.trim().is_empty()
    {
        parts.push(et.trim().to_string());
    }
    if let Some(iso) = exif.iso {
        parts.push(format!("ISO {iso}"));
    }
    if let Some(focal) = exif.focal_length {
        parts.push(format!("{focal:.0}mm"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn format_location(exif: &crate::api_client::ExifInfo) -> Option<String> {
    let parts: Vec<&str> = [&exif.city, &exif.state, &exif.country]
        .into_iter()
        .filter_map(|s| s.as_deref().map(str::trim).filter(|t| !t.is_empty()))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Format a byte count value into a human-readable size string (e.g. KB, MB, GB).
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

/// Truncate a filename to a maximum character limit, appending an ellipsis if needed.
fn truncate_filename(name: &str, max_chars: usize) -> String {
    let count = name.chars().count();
    if count <= max_chars {
        return name.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let head: String = name.chars().take(keep).collect();
    format!("{}…", head)
}

/// Apply zoom to a lightbox Picture. Zoom is fit-relative: 1.0 = the size the
/// texture would occupy inside `viewer` under Contain layout. >1.0 overflows
/// the viewer for panning. Returns the computed content dimensions when zoomed.
fn apply_lightbox_zoom(
    picture: &gtk::Picture,
    viewer: &gtk::ScrolledWindow,
    zoom: f64,
) -> Option<(f64, f64)> {
    if (zoom - 1.0).abs() < 0.001 {
        picture.set_size_request(-1, -1);
        return None;
    }
    let Some(paintable) = picture.paintable() else {
        picture.set_size_request(-1, -1);
        return None;
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
    let cw = fit_w * zoom;
    let ch = fit_h * zoom;
    picture.set_size_request(cw as i32, ch as i32);
    Some((cw, ch))
}

/// Construct and present the fullscreen lightbox view for a selected asset.
pub(super) fn open_lightbox(ui: Rc<LibraryWindowUi>, position: u32) {
    let Some(item) = ui.grid.model.item(position).and_downcast::<AssetObject>() else {
        return;
    };
    let initial_filename = item.property::<String>("filename");
    let title_cap = if ui.split.is_collapsed() { 14 } else { 24 };
    let title_for_header = truncate_filename(&initial_filename, title_cap);

    let page = libadwaita::NavigationPage::builder()
        .title(&title_for_header)
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

    let body = libadwaita::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::End)
        .show_sidebar(false)
        .collapsed(ui.split.is_collapsed())
        .enable_show_gesture(true)
        .enable_hide_gesture(true)
        .min_sidebar_width(180.0)
        .max_sidebar_width(320.0)
        .sidebar_width_fraction(0.4)
        .build();
    let viewer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(8)
        .margin_start(4)
        .margin_end(4)
        .hexpand(true)
        .build();
    // Two picture widgets in a stack so navigation can slide between them.
    let picture_a = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .vexpand(true)
        .hexpand(true)
        .css_classes(["mimick-lightbox-picture"])
        .build();
    let picture_b = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .vexpand(true)
        .hexpand(true)
        .css_classes(["mimick-lightbox-picture"])
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
        .kinetic_scrolling(false)
        .min_content_width(120)
        .build();

    // Spinner overlay: a centered Mimick app icon that rotates while a
    // full-resolution texture is being fetched / decoded. Hidden by default;
    // the load_into_picture closure toggles it.
    let loader_icon = gtk::Image::builder()
        .icon_name("dev.nicx.mimick")
        .pixel_size(72)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["mimick-loader-icon"])
        .build();
    let loader_overlay = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(180)
        .reveal_child(false)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .child(&loader_icon)
        .can_target(false)
        .build();
    let picture_overlay = gtk::Overlay::builder().build();
    picture_overlay.set_child(Some(&scrolled_picture));
    picture_overlay.add_overlay(&loader_overlay);

    let unavailable_title = gtk::Label::builder()
        .label("Preview unavailable")
        .css_classes(["title-3"])
        .build();
    let unavailable_filename = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(42)
        .build();
    let unavailable_mime = gtk::Label::builder().css_classes(["dim-label"]).build();
    let unavailable_open = gtk::Button::builder()
        .label("Open in external app")
        .css_classes(["suggested-action"])
        .build();
    let unavailable_path = Rc::new(RefCell::new(None::<String>));
    unavailable_open.connect_clicked({
        let unavailable_path = unavailable_path.clone();
        move |_| {
            if let Some(path) = unavailable_path.borrow().as_deref() {
                open_local_with_default_app(path);
            }
        }
    });
    let unavailable_card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["mimick-preview-unavailable"])
        .build();
    unavailable_card.append(&unavailable_title);
    unavailable_card.append(&unavailable_filename);
    unavailable_card.append(&unavailable_mime);
    unavailable_card.append(&unavailable_open);
    let unavailable_overlay = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(180)
        .reveal_child(false)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .child(&unavailable_card)
        .can_target(false)
        .build();
    picture_overlay.add_overlay(&unavailable_overlay);

    // Video poster badge: clickable play icon shown over the still thumbnail
    // when the current asset is a video; hands off to the grid's player flow.
    let video_badge_target: Rc<RefCell<Option<(String, String, String)>>> =
        Rc::new(RefCell::new(None));
    let video_badge_icon = gtk::Image::builder()
        .icon_name("mimick-video-symbolic")
        .pixel_size(72)
        .css_classes(vec!["mimick-video-badge".to_string()])
        .build();
    let video_badge_button = gtk::Button::builder()
        .child(&video_badge_icon)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .tooltip_text("Play video in external player")
        .css_classes(vec!["circular".to_string(), "flat".to_string()])
        .visible(false)
        .build();
    video_badge_button.connect_clicked({
        let video_badge_target = video_badge_target.clone();
        let ui_for_video = ui.clone();
        move |_| {
            let Some((local_path, asset_id, filename)) = video_badge_target.borrow().clone() else {
                return;
            };
            if !local_path.is_empty() {
                open_local_with_default_app(&local_path);
            } else {
                spawn_video_handoff(ui_for_video.clone(), asset_id, filename);
            }
        }
    });
    picture_overlay.add_overlay(&video_badge_button);

    let active_a = Rc::new(Cell::new(true));
    let zoom_level = Rc::new(Cell::new(1.0_f64));
    let initial_full = ui.ctx.config.read().data.library_preview_full_resolution;
    let resolution_toggle = gtk::ToggleButton::builder()
        .label(if initial_full { "Raw" } else { "Prev" })
        .tooltip_text("Toggle preview vs original full-resolution image")
        .active(initial_full)
        .build();
    let download = gtk::Button::builder()
        .icon_name("mimick-download-symbolic")
        .tooltip_text("Download asset")
        .build();
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
        .spacing(4)
        .build();
    let actions_spacer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();
    actions.append(&zoom_group);
    actions.append(&actions_spacer);
    actions.append(&resolution_toggle);
    actions.append(&download);
    viewer.append(&picture_overlay);
    viewer.append(&actions);

    let details_inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(10)
        .margin_end(10)
        .build();
    let details_pane = gtk::ScrolledWindow::builder()
        .child(&details_inner)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(false)
        .min_content_width(180)
        .max_content_width(320)
        .css_classes(vec!["mimick-details-pane".to_string()])
        .build();
    let details_filename = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(28)
        .css_classes(vec!["title-3".to_string()])
        .build();
    let details_summary = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(28)
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

    body.set_content(Some(&viewer));
    body.set_sidebar(Some(&details_pane));
    toolbar.set_content(Some(&body));
    page.set_child(Some(&toolbar));

    details_btn
        .bind_property("active", &body, "show-sidebar")
        .sync_create()
        .bidirectional()
        .build();
    ui.split
        .bind_property("collapsed", &body, "collapsed")
        .sync_create()
        .build();

    // On narrow widths, hide the prev/next header buttons -- Left/Right
    // keyboard shortcuts still work, and the saved space lets the title fit.
    // The details toggle stays visible so users can still access the EXIF pane.
    let sync_nav_visibility = {
        let prev_btn = prev_btn.clone();
        let next_btn = next_btn.clone();
        let split = ui.split.clone();
        move || {
            let show = !split.is_collapsed();
            prev_btn.set_visible(show);
            next_btn.set_visible(show);
        }
    };
    sync_nav_visibility();
    let sync_clone = sync_nav_visibility.clone();
    ui.split
        .connect_notify_local(Some("collapsed"), move |_, _| sync_clone());

    let pos_cell = Rc::new(Cell::new(position));
    // Increments on every navigation. Async load tasks capture the generation
    // they were started for and skip UI writes if the user has navigated away
    // by the time their decode finishes (relevant for slow RAW files).
    let load_gen = Rc::new(Cell::new(0u64));
    let load_into_picture = Rc::new({
        let ui = ui.clone();
        let loader_overlay = loader_overlay.clone();
        let unavailable_overlay = unavailable_overlay.clone();
        let unavailable_filename = unavailable_filename.clone();
        let unavailable_mime = unavailable_mime.clone();
        let unavailable_open = unavailable_open.clone();
        let unavailable_path = unavailable_path.clone();
        let load_gen = load_gen.clone();
        let pic_stack = pic_stack.clone();
        let active_a = active_a.clone();
        let video_badge_button = video_badge_button.clone();
        let video_badge_target = video_badge_target.clone();
        move |target: gtk::Picture,
              target_is_a: bool,
              asset_id: String,
              filename: String,
              mime: String,
              local_path: String,
              full_res: bool| {
            let ui = ui.clone();
            let loader = loader_overlay.clone();
            let unavailable_overlay = unavailable_overlay.clone();
            let unavailable_filename = unavailable_filename.clone();
            let unavailable_mime = unavailable_mime.clone();
            let unavailable_open = unavailable_open.clone();
            let unavailable_path = unavailable_path.clone();
            let load_gen = load_gen.clone();
            let pic_stack = pic_stack.clone();
            let active_a = active_a.clone();
            let video_badge_button = video_badge_button.clone();
            let video_badge_target = video_badge_target.clone();
            let our_gen = load_gen.get().wrapping_add(1);
            load_gen.set(our_gen);
            unavailable_overlay.set_reveal_child(false);
            unavailable_overlay.set_can_target(false);
            *unavailable_path.borrow_mut() = None;
            // Videos use the still thumbnail as a poster + play badge; image
            // decoders would all fail and fall through to "unavailable".
            let is_video =
                crate::media_kinds::asset_kind(&mime) == crate::media_kinds::AssetKind::Video;
            log::debug!(
                "Lightbox load: asset={} file={:?} mime={} source={} full_res={} kind={}",
                asset_id,
                filename,
                mime,
                if local_path.is_empty() {
                    "remote"
                } else {
                    "local"
                },
                full_res,
                if is_video { "video" } else { "image" },
            );
            if is_video {
                *video_badge_target.borrow_mut() =
                    Some((local_path.clone(), asset_id.clone(), filename.clone()));
                video_badge_button.set_visible(true);
            } else {
                *video_badge_target.borrow_mut() = None;
                video_badge_button.set_visible(false);
            }
            if let Some(texture) = ui
                .ctx
                .thumbnail_cache
                .get_cached(&asset_id, ThumbnailSize::Preview)
            {
                target.set_paintable(Some(&texture));
            }
            // Reveal the spinner after a short delay so fast cache hits and
            // quick JPEG decodes don't flash it. Local paths get a longer
            // delay since most JPEGs decode in well under 250ms, but RAW
            // and large TIFF decodes can run for seconds and need feedback.
            let arm_delay_ms: u64 = if local_path.is_empty() { 120 } else { 250 };
            let loader_for_arm = loader.clone();
            let cancel_loader = Rc::new(Cell::new(false));
            let cancel_for_arm = cancel_loader.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(arm_delay_ms), move || {
                if !cancel_for_arm.get() {
                    loader_for_arm.set_reveal_child(true);
                }
                glib::ControlFlow::Break
            });
            glib::MainContext::default().spawn_local(async move {
                let is_current = || load_gen.get() == our_gen;
                // Defer the child switch by one idle so the target picture
                // re-measures with the new texture before the slide starts.
                let commit_visible = || {
                    let pic_stack = pic_stack.clone();
                    let active_a = active_a.clone();
                    glib::idle_add_local_once(move || {
                        pic_stack.set_visible_child_name(if target_is_a { "a" } else { "b" });
                        active_a.set(target_is_a);
                    });
                };
                let show_unavailable = |path: Option<String>| {
                    unavailable_filename.set_label(&filename);
                    unavailable_mime.set_label(&mime);
                    unavailable_open.set_visible(path.is_some());
                    *unavailable_path.borrow_mut() = path;
                    unavailable_overlay.set_can_target(true);
                    unavailable_overlay.set_reveal_child(true);
                };
                if is_video {
                    // Use the grid's preview thumbnail as the poster.
                    let thumb_result = ui
                        .ctx
                        .thumbnail_cache
                        .load_thumbnail(&asset_id, ThumbnailSize::Preview)
                        .await;
                    if !is_current() {
                        return;
                    }
                    if let Ok(texture) = thumb_result {
                        target.set_paintable(Some(&texture));
                    }
                    commit_visible();
                    cancel_loader.set(true);
                    loader.set_reveal_child(false);
                    return;
                }
                if !local_path.is_empty() {
                    if let Some(texture) =
                        load_texture_oriented(std::path::Path::new(&local_path)).await
                    {
                        if is_current() {
                            target.set_paintable(Some(&texture));
                            commit_visible();
                            cancel_loader.set(true);
                            loader.set_reveal_child(false);
                        }
                        return;
                    }
                    if asset_id.starts_with(crate::library::LOCAL_ID_PREFIX) {
                        if is_current() {
                            show_unavailable(Some(local_path));
                            commit_visible();
                            cancel_loader.set(true);
                            loader.set_reveal_child(false);
                        }
                        return;
                    }
                }
                if full_res {
                    if let Some(cache_dir) = crate::profile::cache_dir().map(|p| p.join("preview"))
                    {
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let temp = original_preview_cache_path(&cache_dir, &asset_id, &filename);
                        if temp.exists() {
                            log::debug!(
                                "Lightbox original cache hit for {} at {}",
                                asset_id,
                                temp.display(),
                            );
                        } else {
                            let download_started = std::time::Instant::now();
                            let download_result = {
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
                            };
                            if let Err(err) = download_result {
                                log::warn!("Lightbox original fetch failed: {}", err);
                                if is_current() {
                                    show_unavailable(None);
                                    commit_visible();
                                    cancel_loader.set(true);
                                    loader.set_reveal_child(false);
                                }
                                return;
                            }
                            let downloaded_bytes =
                                std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
                            log::debug!(
                                "Lightbox original downloaded for {} in {}ms ({} bytes)",
                                asset_id,
                                download_started.elapsed().as_millis(),
                                downloaded_bytes,
                            );
                        }
                        let decoded = load_texture_oriented(&temp).await;
                        if !is_current() {
                            return;
                        }
                        if let Some(texture) = decoded {
                            target.set_paintable(Some(&texture));
                        } else {
                            show_unavailable(Some(temp.display().to_string()));
                        }
                        commit_visible();
                    } else if is_current() {
                        show_unavailable(None);
                        commit_visible();
                    }
                } else {
                    let thumb_result = ui
                        .ctx
                        .thumbnail_cache
                        .load_thumbnail(&asset_id, ThumbnailSize::Preview)
                        .await;
                    if !is_current() {
                        return;
                    }
                    match thumb_result {
                        Ok(texture) => target.set_paintable(Some(&texture)),
                        Err(_) => show_unavailable(None),
                    }
                    commit_visible();
                }
                if is_current() {
                    cancel_loader.set(true);
                    loader.set_reveal_child(false);
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
        let zoom_group = zoom_group.clone();
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

            let cap = if ui.split.is_collapsed() { 14 } else { 24 };
            page.set_title(&truncate_filename(&filename, cap));
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
            let is_video =
                crate::media_kinds::asset_kind(&mime) == crate::media_kinds::AssetKind::Video;
            resolution_toggle.set_visible(!is_local && !is_video);
            download.set_visible(!is_local && !is_video);
            zoom_group.set_visible(!is_video);

            // Load into the *inactive* picture; commit the slide transition
            // after the texture is set so the user sees the current image
            // (with loader spinner) until the new one is actually ready.
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
                target_is_a,
                asset_id.clone(),
                filename.clone(),
                mime.clone(),
                local_path.clone(),
                resolution_toggle.is_active(),
            );
            nav_dir.set(0);

            if is_local
                && crate::media_kinds::asset_kind(&mime) != crate::media_kinds::AssetKind::Video
            {
                // Local image: parse EXIF on a blocking worker.
                // Hits the on-disk cache so repeat opens are cheap.
                details_loading.set_visible(true);
                let pos_cell_async = pos_cell.clone();
                let details_loading = details_loading.clone();
                let details_exif = details_exif.clone();
                let local_path_async = local_path.clone();
                glib::MainContext::default().spawn_local(async move {
                    let cache_root = local_exif::cache_root();
                    let path_for_blocking = local_path_async.clone();
                    let probed = tokio::task::spawn_blocking(move || {
                        let path = std::path::Path::new(&path_for_blocking);
                        let meta = std::fs::metadata(path).ok();
                        let file_size = meta.as_ref().map(|m| m.len());
                        let mtime_iso = meta
                            .as_ref()
                            .and_then(|m| m.modified().ok())
                            .map(systemtime_to_rfc3339);
                        // Pixbuf header-only read — covers JPEG, PNG, GIF,
                        // TIFF, WebP and HEIF/AVIF (when loaders installed).
                        let dims = gtk::gdk_pixbuf::Pixbuf::file_info(path)
                            .map(|(_, w, h)| (w, h))
                            .and_then(|(w, h)| {
                                let w = u32::try_from(w).ok()?;
                                let h = u32::try_from(h).ok()?;
                                Some((w, h))
                            });
                        let exif = local_exif::load_or_extract(&cache_root, path);
                        LocalProbe {
                            exif,
                            file_size,
                            mtime_iso,
                            dims,
                        }
                    })
                    .await
                    .ok();
                    if pos_cell_async.get() != pos {
                        return;
                    }
                    details_loading.set_visible(false);
                    // Render whatever we have. Files without an EXIF block
                    // (Unsplash, screenshots, edited copies) still get the
                    // Image group populated from filesystem + Pixbuf, so the
                    // user always sees *something* in the details pane.
                    let Some(probe) = probed else {
                        return;
                    };
                    if probe.exif.is_none()
                        && probe.file_size.is_none()
                        && probe.dims.is_none()
                        && probe.mtime_iso.is_none()
                    {
                        return;
                    }
                    let mut info = probe
                        .exif
                        .as_ref()
                        .map_or_else(LocalExif::default, Clone::clone);
                    if info.image_width.is_none() {
                        info.image_width = probe.dims.map(|(w, _)| w);
                    }
                    if info.image_height.is_none() {
                        info.image_height = probe.dims.map(|(_, h)| h);
                    }
                    let used_mtime_fallback = info.date_time_original.is_none();
                    if used_mtime_fallback {
                        info.date_time_original = probe.mtime_iso.clone();
                    }
                    let taken_label = if used_mtime_fallback {
                        "Modified"
                    } else {
                        "Taken"
                    };
                    let projected = exif_from_local(&info, probe.file_size);
                    fill_exif_box(&details_exif, &projected, taken_label);
                    details_exif.set_visible(true);
                });
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
                    fill_exif_box(&details_exif, &exif, "Taken");
                    details_exif.set_visible(true);
                }
            });
        }
    });

    (*render)();

    let goto_prev = Rc::new(clone!(
        #[strong]
        pos_cell,
        #[strong]
        render,
        #[strong]
        nav_dir,
        move || {
            let pos = pos_cell.get();
            if pos > 0 {
                pos_cell.set(pos - 1);
                nav_dir.set(-1);
                (*render)();
            }
        }
    ));
    prev_btn.connect_clicked(clone!(
        #[strong]
        goto_prev,
        move |_| (*goto_prev)()
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
            let content = apply_lightbox_zoom(&active_picture(), &scrolled_picture, z_new);
            zoom_reset_btn.set_label(&format!("{}%", (z_new * 100.0).round() as i32));

            // Pre-set adjustment ranges to match the new content size so the
            // scroll position can be applied in the same frame. Without this
            // the value would be clamped to the stale (old-zoom) range and
            // corrected only after layout, causing a one-frame flicker.
            if let Some((cw, ch)) = content {
                hadj.set_upper(cw.max(viewer_w));
                hadj.set_page_size(viewer_w);
                vadj.set_upper(ch.max(viewer_h));
                vadj.set_page_size(viewer_h);
            } else {
                hadj.set_upper(viewer_w);
                hadj.set_page_size(viewer_w);
                vadj.set_upper(viewer_h);
                vadj.set_page_size(viewer_h);
            }
            hadj.set_value(target_scroll_x);
            vadj.set_value(target_scroll_y);
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

    // Trackpad pinch-to-zoom. On scrolled_picture so it shares a stable
    // coordinate frame with drag (see drag comment below).
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
    scrolled_picture.add_controller(pinch.clone());

    // Click-and-drag panning when zoomed in. Attached to scrolled_picture,
    // not pic_stack: pic_stack moves under the cursor when we update the
    // scroll adjustments, which makes the gesture's pic_stack-local offset
    // oscillate frame-to-frame and jitter the image.
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
    scrolled_picture.add_controller(drag.clone());
    drag.group_with(&pinch);

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

    // Horizontal swipe for prev/next navigation; ignored when zoomed in.
    let swipe = gtk::GestureSwipe::new();
    swipe.set_touch_only(false);
    swipe.connect_swipe(clone!(
        #[strong]
        goto_prev,
        #[strong]
        goto_next,
        #[strong]
        zoom_level,
        move |_, vx, _vy| {
            // Ignore swipes when zoomed in — those should pan instead.
            if (zoom_level.get() - 1.0).abs() > 0.01 {
                return;
            }
            // vx < 0 means finger moved left → go to next asset.
            // vx > 0 means finger moved right → go to previous asset.
            const MIN_VELOCITY: f64 = 50.0;
            if vx < -MIN_VELOCITY {
                (*goto_next)();
            } else if vx > MIN_VELOCITY {
                (*goto_prev)();
            }
        }
    ));
    pic_stack.add_controller(swipe);

    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed(clone!(
        #[strong]
        ui,
        #[strong]
        details_btn,
        #[strong]
        goto_prev,
        #[strong]
        goto_next,
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
                    (*goto_prev)();
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
            btn.set_label(if btn.is_active() { "Raw" } else { "Prev" });
            (*render)();
        }
    ));

    ui.nav.push(&page);
}

#[cfg(test)]
mod tests {
    use super::original_preview_cache_path;

    #[test]
    fn original_preview_cache_path_keeps_decoder_extension() {
        let cache = std::path::Path::new("/tmp/previews");
        assert_eq!(
            original_preview_cache_path(cache, "remote-id", "PXL_20250516_222429137.dng"),
            cache.join("remote-id.dng")
        );
        assert_eq!(
            original_preview_cache_path(cache, "remote-id", "extensionless"),
            cache.join("remote-id.bin")
        );
    }
}
