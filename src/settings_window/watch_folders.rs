//! Watch-folder row construction, folder-rules dialog, and album picker dialog.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use glib::clone;
use gtk::prelude::*;
use gtk::{Box, Button, Entry, ListBox, Orientation};
use libadwaita as adw;

use crate::config::{FolderRules, FolderSyncMethod, StartupCatchupMode};
use crate::watch_path_display::{display_watch_path, watch_path_subtitle};

use super::{DEFAULT_ALBUM_LABEL, FolderRowData, WatchPathEntry};

pub(super) fn add_folder_row(
    list: &ListBox,
    entry: &WatchPathEntry,
    fallback_catchup_mode: StartupCatchupMode,
    albums_ref: Rc<RefCell<Vec<(String, String)>>>,
    tracked_rows: &Rc<RefCell<Vec<FolderRowData>>>,
    on_settings_changed: Rc<dyn Fn()>,
) {
    let path = entry.path().to_string();
    let base_subtitle = watch_path_subtitle(&path).unwrap_or_default().to_string();

    let mut initial_subtitle = base_subtitle.clone();
    if !initial_subtitle.is_empty() {
        initial_subtitle.push('\n');
    }
    initial_subtitle.push_str("Status: Idle");

    let expander_row = adw::ExpanderRow::builder()
        .title(display_watch_path(&path))
        .subtitle(&initial_subtitle)
        .subtitle_lines(2)
        .title_lines(1)
        .build();

    let album_name = Rc::new(RefCell::new(
        entry
            .album_name()
            .unwrap_or(DEFAULT_ALBUM_LABEL)
            .to_string(),
    ));

    let rules = Rc::new(RefCell::new(entry.rules()));

    let picker_btn = Button::builder()
        .label(format!("Album: {}", album_name.borrow()))
        .valign(gtk::Align::Center)
        .tooltip_text("Select or create a target Immich album")
        .build();

    let picker_btn_clone = picker_btn.clone();
    let album_name_clone = album_name.clone();
    let albums_ref_clone = albums_ref.clone();
    let on_settings_changed_for_picker = on_settings_changed.clone();

    picker_btn.connect_clicked(clone!(
        #[weak]
        expander_row,
        move |_| {
            if let Some(window) = expander_row
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok())
            {
                let window_clone = window.clone();
                let albums_ref_clone = albums_ref_clone.clone();
                let album_name_clone = album_name_clone.clone();
                let picker_btn_clone = picker_btn_clone.clone();
                let on_settings_changed_for_picker = on_settings_changed_for_picker.clone();

                glib::idle_add_local_once(move || {
                    show_album_picker_dialog(
                        &window_clone,
                        albums_ref_clone,
                        album_name_clone,
                        picker_btn_clone,
                        on_settings_changed_for_picker,
                    );
                });
            }
        }
    ));

    let album_subrow = adw::ActionRow::builder().title("Target Album").build();
    album_subrow.add_suffix(&picker_btn);
    expander_row.add_row(&album_subrow);

    let remove_btn = Button::builder()
        .icon_name("user-trash-symbolic")
        .valign(gtk::Align::Center)
        .css_classes(vec!["destructive-action".to_string()])
        .build();
    let rules_btn = Button::builder()
        .label("Rules")
        .tooltip_text("Edit folder rules")
        .valign(gtk::Align::Center)
        .build();

    let list_clone = list.clone();
    let tracked_clone = tracked_rows.clone();
    let path_clone = path.clone();
    let rules_clone = rules.clone();
    let path_for_rules = path.clone();
    let on_settings_changed_for_rules = on_settings_changed.clone();
    let fallback_catchup_mode_for_rules = fallback_catchup_mode.clone();

    rules_btn.connect_clicked(clone!(
        #[weak]
        expander_row,
        move |_| {
            if let Some(window) = expander_row
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok())
            {
                let window = window.clone();
                let path_for_rules = path_for_rules.clone();
                let rules_clone = rules_clone.clone();
                let on_settings_changed_for_rules = on_settings_changed_for_rules.clone();
                let fallback_catchup_mode_for_rules = fallback_catchup_mode_for_rules.clone();
                glib::idle_add_local_once(move || {
                    show_folder_rules_dialog(
                        &window,
                        &path_for_rules,
                        fallback_catchup_mode_for_rules.clone(),
                        rules_clone,
                        on_settings_changed_for_rules,
                    );
                });
            }
        }
    ));

    let on_settings_changed_for_remove = on_settings_changed.clone();

    remove_btn.connect_clicked(clone!(
        #[weak]
        expander_row,
        move |_| {
            let list_clone = list_clone.clone();
            let tracked_clone = tracked_clone.clone();
            let path_clone = path_clone.clone();
            let expander_row = expander_row.clone();
            let on_settings_changed_for_remove = on_settings_changed_for_remove.clone();
            glib::idle_add_local_once(move || {
                if let Some(focus_target) = list_clone.first_child() {
                    focus_target.grab_focus();
                }
                list_clone.remove(&expander_row);
                tracked_clone.borrow_mut().retain(|r| r.path != path_clone);
                (on_settings_changed_for_remove)();
            });
        }
    ));

    let rules_subrow = adw::ActionRow::builder().title("Folder Rules").build();
    rules_subrow.add_suffix(&rules_btn);
    expander_row.add_row(&rules_subrow);

    let remove_subrow = adw::ActionRow::builder().title("Remove Folder").build();
    remove_subrow.add_suffix(&remove_btn);
    expander_row.add_row(&remove_subrow);

    list.append(&expander_row);
    tracked_rows.borrow_mut().push(FolderRowData {
        path,
        album_name,
        rules,
        action_row: expander_row,
        base_subtitle,
    });
}

fn show_folder_rules_dialog(
    parent: &impl gtk::prelude::IsA<gtk::Window>,
    folder_path: &str,
    fallback_catchup_mode: StartupCatchupMode,
    rules_state: Rc<RefCell<FolderRules>>,
    on_settings_changed: Rc<dyn Fn()>,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Folder Rules")
        .default_width(420)
        .build();
    let content = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    dialog.set_content(Some(&content));

    let title = gtk::Label::builder()
        .label(format!("Rules for {}", display_watch_path(folder_path)))
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    content.append(&title);

    let current = rules_state.borrow().clone();

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec![String::from("boxed-list")])
        .build();

    let ignore_hidden = adw::SwitchRow::builder()
        .title("Ignore Hidden Files / Folders")
        .subtitle("Skip paths that contain hidden components such as .cache or .thumbnails.")
        .active(current.ignore_hidden)
        .build();

    let sync_model = gtk::StringList::new(&[
        "Full Sync",
        "Only Upload from Folder",
        "Only Download to Folder",
    ]);
    let sync_method = adw::ComboRow::builder()
        .title("Sync Method")
        .subtitle("Controls which direction this folder syncs.")
        .model(&sync_model)
        .build();
    sync_method.set_selected(match current.sync_method {
        FolderSyncMethod::Full => 0,
        FolderSyncMethod::UploadOnly => 1,
        FolderSyncMethod::DownloadOnly => 2,
    });

    let startup_model = gtk::StringList::new(&["Full Scan", "Recent Only (7d)", "New Files Only"]);
    let startup_scan = adw::ComboRow::builder()
        .title("Startup Scan")
        .subtitle("Controls how this folder is scanned when Mimick starts.")
        .model(&startup_model)
        .build();
    startup_scan.set_selected(
        match current
            .startup_catchup_mode
            .clone()
            .unwrap_or(fallback_catchup_mode)
        {
            StartupCatchupMode::Full => 0,
            StartupCatchupMode::RecentOnly => 1,
            StartupCatchupMode::NewFilesOnly => 2,
        },
    );

    let delete_folder_to_album = adw::SwitchRow::builder()
        .title("Mirror Folder Deletions to Album")
        .subtitle("When a synced folder file is gone, move the matching Immich asset to trash.")
        .active(current.delete_folder_to_album)
        .build();
    let delete_album_to_folder = adw::SwitchRow::builder()
        .title("Mirror Album Deletions to Folder")
        .subtitle("Currently unavailable — waiting on an upstream Flatpak Trash portal fix. The setting stays off until then.")
        .active(false)
        .sensitive(false)
        .build();

    list_box.append(&sync_method);
    list_box.append(&startup_scan);
    list_box.append(&delete_folder_to_album);
    list_box.append(&delete_album_to_folder);
    list_box.append(&ignore_hidden);
    content.append(&list_box);

    let max_size_entry = Entry::builder()
        .placeholder_text("Max file size in MB, leave blank for no limit")
        .width_request(0)
        .text(
            current
                .max_file_size_mb
                .map(|value| value.to_string())
                .unwrap_or_default(),
        )
        .build();
    content.append(&max_size_entry);

    let extensions_entry = Entry::builder()
        .placeholder_text("Comma-separated extensions, e.g. jpg,png,mp4")
        .width_request(0)
        .text(current.allowed_extensions.join(", "))
        .build();
    content.append(&extensions_entry);

    let actions = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel_btn = Button::builder().label("Cancel").build();
    let save_btn = Button::builder()
        .label("Save")
        .css_classes(vec!["suggested-action".to_string()])
        .build();
    actions.append(&cancel_btn);
    actions.append(&save_btn);
    content.append(&actions);

    cancel_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    save_btn.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| {
            let max_file_size_mb = max_size_entry.text().trim().parse::<u64>().ok();
            let allowed_extensions = extensions_entry
                .text()
                .split(',')
                .map(|part| part.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();

            // delete_album_to_folder UI is forced off / insensitive while
            // album-to-folder mirroring is disabled.
            let stored_delete_album_to_folder = rules_state.borrow().delete_album_to_folder;
            *rules_state.borrow_mut() = FolderRules {
                ignore_hidden: ignore_hidden.is_active(),
                max_file_size_mb,
                allowed_extensions,
                sync_method: match sync_method.selected() {
                    1 => FolderSyncMethod::UploadOnly,
                    2 => FolderSyncMethod::DownloadOnly,
                    _ => FolderSyncMethod::Full,
                },
                startup_catchup_mode: Some(match startup_scan.selected() {
                    1 => StartupCatchupMode::RecentOnly,
                    2 => StartupCatchupMode::NewFilesOnly,
                    _ => StartupCatchupMode::Full,
                }),
                delete_folder_to_album: delete_folder_to_album.is_active(),
                delete_album_to_folder: stored_delete_album_to_folder,
            };
            (on_settings_changed)();
            dialog.close();
        }
    ));

    dialog.present();
}
pub(super) fn show_album_picker_dialog(
    parent: &impl gtk::prelude::IsA<gtk::Window>,
    albums_ref: Rc<RefCell<Vec<(String, String)>>>,
    target_album_state: Rc<RefCell<String>>,
    trigger_btn: Button,
    on_settings_changed: Rc<dyn Fn()>,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Select Album")
        .default_width(400)
        .default_height(500)
        .build();

    let header_bar = adw::HeaderBar::new();
    let vbox = Box::builder().orientation(Orientation::Vertical).build();
    dialog.set_content(Some(&vbox));
    vbox.append(&header_bar);

    let search_entry = gtk::SearchEntry::builder()
        .halign(gtk::Align::Center)
        .width_request(300)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    vbox.append(&search_entry);

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();
    list_box.add_css_class("boxed-list");

    let scrolled_window = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    scrolled_window.set_child(Some(&list_box));
    vbox.append(&scrolled_window);

    // Dynamic filtering capability
    let albums_ref_cloned = albums_ref.clone();
    let apply_filter = {
        let list_box = list_box.clone();
        let dialog = dialog.clone();
        let target_album_state = target_album_state.clone();
        let trigger_btn = trigger_btn.clone();

        move |query: &str| {
            // Clear existing
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let q = query.trim().to_lowercase();

            // Row 1: Default Folder Name (only if it matches search)
            if q.is_empty() || DEFAULT_ALBUM_LABEL.to_lowercase().contains(&q) {
                let default_row = adw::ActionRow::builder()
                    .title(DEFAULT_ALBUM_LABEL)
                    .subtitle("Creates album dynamically per-folder")
                    .activatable(true)
                    .build();
                let dialog_clone = dialog.clone();
                let state_clone = target_album_state.clone();
                let btn_clone = trigger_btn.clone();
                let on_settings_changed_clone = on_settings_changed.clone();
                default_row.connect_activated(move |_| {
                    *state_clone.borrow_mut() = DEFAULT_ALBUM_LABEL.to_string();
                    btn_clone.set_label(&format!("Album: {}", DEFAULT_ALBUM_LABEL));
                    (on_settings_changed_clone)();
                    dialog_clone.close();
                });
                list_box.append(&default_row);
            }

            // Row 2: Create Custom (if query is typed)
            if !q.is_empty() {
                let typed_raw = query.trim().to_string();
                let create_row = adw::ActionRow::builder()
                    .title(format!("Create new: \"{}\"", typed_raw))
                    .activatable(true)
                    .build();
                let dialog_clone = dialog.clone();
                let state_clone = target_album_state.clone();
                let btn_clone = trigger_btn.clone();
                let on_settings_changed_clone = on_settings_changed.clone();
                create_row.connect_activated(move |_| {
                    *state_clone.borrow_mut() = typed_raw.clone();
                    btn_clone.set_label(&format!("Album: {}", typed_raw));
                    (on_settings_changed_clone)();
                    dialog_clone.close();
                });
                list_box.append(&create_row);
            }

            // Row 3+: Remote Albums
            for (name, _) in albums_ref_cloned.borrow().iter() {
                if name == DEFAULT_ALBUM_LABEL {
                    continue; // Skip the "Use default folder name" if we pushed it above
                }
                if q.is_empty() || name.to_lowercase().contains(&q) {
                    let album_name = name.clone();
                    let row = adw::ActionRow::builder()
                        .title(&album_name)
                        .activatable(true)
                        .build();
                    let dialog_clone = dialog.clone();
                    let state_clone = target_album_state.clone();
                    let btn_clone = trigger_btn.clone();
                    let album_name_clone = album_name.clone();
                    let on_settings_changed_clone = on_settings_changed.clone();
                    row.connect_activated(move |_| {
                        *state_clone.borrow_mut() = album_name_clone.clone();
                        btn_clone.set_label(&format!("Album: {}", album_name_clone));
                        (on_settings_changed_clone)();
                        dialog_clone.close();
                    });
                    list_box.append(&row);
                }
            }
        }
    };

    // Initial populate
    apply_filter("");

    let apply_filter_rc = Rc::new(apply_filter);
    search_entry.connect_search_changed(move |entry| {
        apply_filter_rc(&entry.text());
    });

    dialog.present();
}
