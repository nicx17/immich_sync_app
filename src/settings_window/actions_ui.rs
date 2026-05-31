use adw::prelude::*;
use gtk::Button;
use libadwaita as adw;

pub struct ActionsWidgets {
    pub sync_now_btn: Button,
    pub pause_btn: Button,
    pub queue_btn: Button,
    pub export_btn: Button,
    pub clear_cache_btn: Button,
    pub quit_btn: Button,
}

pub fn build_actions_group(
    status_page: &adw::PreferencesPage,
    settings_page: &adw::PreferencesPage,
) -> ActionsWidgets {
    let controls_group = adw::PreferencesGroup::builder().title("Actions").build();
    status_page.add(&controls_group);

    // FlowBox so buttons wrap automatically on narrow widths
    let actions_flow = gtk::FlowBox::builder()
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(4)
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    controls_group.add(&actions_flow);

    let sync_now_btn = Button::builder()
        .label("Sync Now")
        .css_classes(vec!["suggested-action".to_string()])
        .hexpand(true)
        .build();
    actions_flow.insert(&sync_now_btn, -1);

    let pause_btn = Button::builder().label("Pause").hexpand(true).build();
    actions_flow.insert(&pause_btn, -1);

    let queue_btn = Button::builder()
        .label("Queue Inspector")
        .hexpand(true)
        .build();
    actions_flow.insert(&queue_btn, -1);

    let export_btn = Button::builder()
        .label("Export Diagnostics")
        .hexpand(true)
        .build();
    actions_flow.insert(&export_btn, -1);

    let clear_cache_btn = Button::builder()
        .label("Clear Cache")
        .tooltip_text(
            "Removes all on-disk caches: thumbnails, decoded RAW previews, \
             EXIF, video, and preview files.",
        )
        .hexpand(true)
        .build();
    actions_flow.insert(&clear_cache_btn, -1);

    let app_group = adw::PreferencesGroup::builder()
        .title("Application")
        .build();
    settings_page.add(&app_group);

    let app_flow = gtk::FlowBox::builder()
        .homogeneous(false)
        .min_children_per_line(1)
        .max_children_per_line(2)
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    app_group.add(&app_flow);

    let quit_btn = Button::builder()
        .label("Quit")
        .css_classes(vec!["destructive-action".to_string()])
        .halign(gtk::Align::Start)
        .hexpand(false)
        .width_request(120)
        .build();
    app_flow.insert(&quit_btn, -1);

    ActionsWidgets {
        sync_now_btn,
        pause_btn,
        queue_btn,
        export_btn,
        clear_cache_btn,
        quit_btn,
    }
}
