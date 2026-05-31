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

    let actions_flow = action_flow(true, 4);
    controls_group.add(&actions_flow);

    let sync_now_btn = flow_button(&actions_flow, "Sync Now", Some("suggested-action"), None);
    let pause_btn = flow_button(&actions_flow, "Pause", None, None);
    let queue_btn = flow_button(&actions_flow, "Queue Inspector", None, None);
    let export_btn = flow_button(&actions_flow, "Export Diagnostics", None, None);
    let clear_cache_btn = flow_button(
        &actions_flow,
        "Clear Cache",
        None,
        Some(
            "Removes all on-disk caches: thumbnails, decoded RAW previews, \
             EXIF, video, and preview files.",
        ),
    );

    let app_group = adw::PreferencesGroup::builder()
        .title("Application")
        .build();
    settings_page.add(&app_group);

    let app_flow = action_flow(false, 2);
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

fn action_flow(homogeneous: bool, max_children_per_line: u32) -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .homogeneous(homogeneous)
        .min_children_per_line(1)
        .max_children_per_line(max_children_per_line)
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(8)
        .column_spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .build()
}

fn flow_button(
    flow: &gtk::FlowBox,
    label: &str,
    css_class: Option<&str>,
    tooltip: Option<&str>,
) -> Button {
    let mut builder = Button::builder().label(label).hexpand(true);
    if let Some(css_class) = css_class {
        builder = builder.css_classes(vec![css_class.to_string()]);
    }
    if let Some(tooltip) = tooltip {
        builder = builder.tooltip_text(tooltip);
    }
    let button = builder.build();
    flow.insert(&button, -1);
    button
}
