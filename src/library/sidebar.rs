//! Album sidebar for the library view with album listing and selection.

use gtk::prelude::*;

pub struct SidebarParts {
    pub root: gtk::Box,
    pub refresh_button: gtk::Button,
    pub delete_button: gtk::Button,
    pub list: gtk::ListBox,
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

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .vexpand(true)
        .build();

    root.append(&actions);
    root.append(&list);

    SidebarParts {
        root,
        refresh_button,
        delete_button,
        list,
    }
}
