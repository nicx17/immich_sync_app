use gtk::prelude::*;
use libadwaita::prelude::*;

pub struct SidebarParts {
    pub root: gtk::Box,
    pub refresh_button: gtk::Button,
    pub delete_button: gtk::Button,
    /// Fixed destinations: index 0 = Photos (Timeline), 1 = Explore (random).
    pub fixed_list: gtk::ListBox,
    /// Album list, populated dynamically by `reload_sidebar`.
    pub albums_list: gtk::ListBox,
}

pub fn build_sidebar() -> SidebarParts {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .width_request(260)
        .build();

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let refresh_button = gtk::Button::builder()
        .label("Refresh")
        .icon_name("view-refresh-symbolic")
        .hexpand(true)
        .build();
    let delete_button = gtk::Button::builder()
        .label("Delete Album")
        .icon_name("user-trash-symbolic")
        .sensitive(false)
        .hexpand(true)
        .build();

    actions.append(&refresh_button);
    actions.append(&delete_button);

    let fixed_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();

    // `image-symbolic` ships in some distros' icon themes but not the
    // upstream Adwaita set, so the row rendered with no glyph. Use names
    // guaranteed by `adwaita-icon-theme`.
    fixed_list.append(&action_row(
        "Photos",
        "Timeline of every photo and video",
        "image-x-generic-symbolic",
        "photos",
    ));
    fixed_list.append(&action_row(
        "Explore",
        "People, places, and things",
        "view-grid-symbolic",
        "explore",
    ));

    let albums_header = gtk::Label::builder()
        .label("Albums")
        .xalign(0.0)
        .css_classes(vec!["heading".to_string(), "dim-label".to_string()])
        .margin_top(6)
        .build();

    let albums_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();

    let albums_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&albums_list)
        .build();

    root.append(&actions);
    root.append(&fixed_list);
    root.append(&albums_header);
    root.append(&albums_scroll);

    SidebarParts {
        root,
        refresh_button,
        delete_button,
        fixed_list,
        albums_list,
    }
}

/// Destination row whose `tooltip_text` carries the source key the
/// row-selected handler in `mod.rs` dispatches from.
fn action_row(title: &str, subtitle: &str, icon_name: &str, key: &str) -> gtk::ListBoxRow {
    let row = libadwaita::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    let icon = gtk::Image::from_icon_name(icon_name);
    row.add_prefix(&icon);
    gtk::ListBoxRow::builder()
        .tooltip_text(key)
        .child(&row)
        .build()
}
