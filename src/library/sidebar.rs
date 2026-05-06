use gtk::prelude::*;
use libadwaita::prelude::*;

pub struct SidebarParts {
    pub root: gtk::Box,
    pub connection_row: libadwaita::ActionRow,
    pub server_row: libadwaita::ActionRow,
    pub fixed_list: gtk::ListBox,
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

    let fixed_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();

    let connection_row = libadwaita::ActionRow::builder()
        .title("Connection")
        .subtitle("Offline")
        .build();
    let server_row = libadwaita::ActionRow::builder()
        .title("Server")
        .subtitle("Statistics unavailable")
        .build();
    let connection_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    connection_list.append(&gtk::ListBoxRow::builder().child(&connection_row).build());
    connection_list.append(&gtk::ListBoxRow::builder().child(&server_row).build());

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
    fixed_list.append(&action_row(
        "Albums",
        "Recent, owned, and shared",
        "folder-pictures-symbolic",
        "albums",
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

    root.append(&connection_list);
    root.append(&fixed_list);
    root.append(&albums_header);
    root.append(&albums_scroll);

    SidebarParts {
        root,
        connection_row,
        server_row,
        fixed_list,
        albums_list,
    }
}

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
