use gtk::prelude::*;
use libadwaita::prelude::*;

pub struct SidebarParts {
    pub root: gtk::Box,
    pub refresh_button: gtk::Button,
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

    let refresh_button = gtk::Button::builder()
        .label("Refresh")
        .icon_name("view-refresh-symbolic")
        .hexpand(true)
        .build();

    let fixed_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();

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

    root.append(&refresh_button);
    root.append(&fixed_list);
    root.append(&albums_header);
    root.append(&albums_scroll);

    SidebarParts {
        root,
        refresh_button,
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
