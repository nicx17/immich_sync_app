use adw::prelude::*;
use gtk::{Button, Entry, PasswordEntry, Switch};
use libadwaita as adw;

pub struct ConnectivityWidgets {
    pub internal_switch: Switch,
    pub external_switch: Switch,
    pub internal_entry: Entry,
    pub external_entry: Entry,
    pub api_key_entry: PasswordEntry,
    pub test_btn: Button,
    pub save_btn: Button,
}

pub fn build_connectivity_group(
    settings_page: &adw::PreferencesPage,
    window: &adw::ApplicationWindow,
) -> ConnectivityWidgets {
    // --- CONNECTIVITY GROUP ---
    let conn_group = adw::PreferencesGroup::builder()
        .title("Connectivity")
        .build();
    settings_page.add(&conn_group);

    // Internal URL
    let internal_row = adw::ActionRow::builder()
        .title("Internal URL (LAN)")
        .title_lines(1)
        .build();
    let internal_switch = Switch::builder().valign(gtk::Align::Center).build();
    let internal_entry = Entry::builder()
        .placeholder_text("http://…")
        .valign(gtk::Align::Center)
        .width_request(140)
        .max_width_chars(16)
        .hexpand(true)
        .build();
    internal_row.add_prefix(&internal_switch);
    internal_row.add_suffix(&internal_entry);
    conn_group.add(&internal_row);

    // External URL
    let external_row = adw::ActionRow::builder()
        .title("External URL (WAN)")
        .title_lines(1)
        .build();
    let external_switch = Switch::builder().valign(gtk::Align::Center).build();
    let external_entry = Entry::builder()
        .placeholder_text("https://…")
        .valign(gtk::Align::Center)
        .width_request(140)
        .max_width_chars(16)
        .hexpand(true)
        .build();
    external_row.add_prefix(&external_switch);
    external_row.add_suffix(&external_entry);
    conn_group.add(&external_row);

    // API Key
    let api_key_row = adw::ActionRow::builder().title("API Key").build();
    let api_key_entry = PasswordEntry::builder()
        .valign(gtk::Align::Center)
        .width_request(140)
        .max_width_chars(16)
        .hexpand(true)
        .build();
    api_key_row.add_suffix(&api_key_entry);
    conn_group.add(&api_key_row);

    // Test Connection Button
    let test_btn = Button::builder()
        .label("Test Connection")
        .margin_top(12)
        .build();
    conn_group.add(&test_btn);

    let save_btn = Button::builder()
        .label("Save Credentials")
        .css_classes(vec!["suggested-action".to_string()])
        .margin_top(6)
        .build();
    conn_group.add(&save_btn);

    let settings_breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 500sp").expect("valid breakpoint condition"),
    );
    settings_breakpoint.add_setter(&internal_row, "title", Some(&"LAN URL".to_value()));
    settings_breakpoint.add_setter(&external_row, "title", Some(&"WAN URL".to_value()));
    settings_breakpoint.add_setter(&internal_entry, "width-request", Some(&140i32.to_value()));
    settings_breakpoint.add_setter(&external_entry, "width-request", Some(&140i32.to_value()));
    settings_breakpoint.add_setter(&api_key_entry, "width-request", Some(&140i32.to_value()));
    window.add_breakpoint(settings_breakpoint);

    ConnectivityWidgets {
        internal_switch,
        external_switch,
        internal_entry,
        external_entry,
        api_key_entry,
        test_btn,
        save_btn,
    }
}
