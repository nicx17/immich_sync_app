//! Server statistics dialog.
//!
//! Presented when the user clicks the bottom-sidebar connection or server row.
//! Pulls `/api/server/statistics` (admin) plus `/api/assets/statistics` (per-user)
//! and `/api/server/about` to render a single summary view.

use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::app_context::AppContext;

/// Present the server statistics dialog to the user.
///
/// Fetches both server-wide and user-specific usage statistics, along with
/// the server version, and displays them in a modal dialog.
pub fn present(ctx: Arc<AppContext>, parent: &libadwaita::ApplicationWindow) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Server statistics")
        .body("")
        .content_width(280)
        .build();
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .build();

    let loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    let spinner = gtk::Spinner::builder().spinning(true).build();
    let loading_label = gtk::Label::new(Some("Loading\u{2026}"));
    loading.append(&spinner);
    loading.append(&loading_label);
    content.append(&loading);

    dialog.set_extra_child(Some(&content));
    dialog.present(Some(parent));

    let content = Rc::new(content);
    glib::MainContext::default().spawn_local({
        let ctx = ctx.clone();
        let content = content.clone();
        async move {
            let server_stats = ctx.api_client.fetch_server_statistics().await;
            let asset_stats = ctx.api_client.fetch_server_stats().await;
            let about = ctx.api_client.fetch_server_about().await;

            while let Some(child) = content.first_child() {
                content.remove(&child);
            }

            if let Ok(about) = &about {
                append_version_badge(&content, &about.version);
            }

            match (&server_stats, &asset_stats) {
                (Ok(stats), _) => append_server_statistics(&content, stats),
                (Err(_), Ok(assets)) => append_asset_statistics(&content, assets),
                (Err(server_err), Err(asset_err)) => {
                    append_error_ui(&content, asset_err, server_err)
                }
            }
        }
    });
}

fn append_version_badge(content: &gtk::Box, version: &str) {
    let badge_label = gtk::Label::builder()
        .label(format!("Immich {}", version))
        .css_classes(vec!["caption".to_string()])
        .build();
    let badge = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .css_classes(vec!["mimick-version-badge".to_string()])
        .build();
    badge.append(&badge_label);
    content.append(&badge);
}

fn append_server_statistics(content: &gtk::Box, stats: &crate::api_client::ServerStatistics) {
    let cards = stat_card_row(&[
        ("Photos", format_number(stats.photos), "photo-card"),
        ("Videos", format_number(stats.videos), "video-card"),
        ("Storage", format_size(stats.usage), "storage-card"),
    ]);
    content.append(&cards);

    if !stats.usage_by_user.is_empty() {
        let user_group = libadwaita::PreferencesGroup::builder()
            .title("By user")
            .build();
        for user in &stats.usage_by_user {
            let row = user_row(user);
            user_group.add(&row);
        }
        content.append(&user_group);
    }
}

fn append_asset_statistics(content: &gtk::Box, asset_stats: &crate::api_client::ServerStats) {
    let cards = stat_card_row(&[
        ("Photos", format_number(asset_stats.images), "photo-card"),
        ("Videos", format_number(asset_stats.videos), "video-card"),
        ("Total", format_number(asset_stats.total), "storage-card"),
    ]);
    content.append(&cards);

    let note = gtk::Label::builder()
        .label("Server-wide statistics require administrator access.")
        .css_classes(vec!["dim-label".to_string(), "caption".to_string()])
        .wrap(true)
        .halign(gtk::Align::Center)
        .build();
    content.append(&note);
}

fn append_error_ui(content: &gtk::Box, asset_err: &str, server_err: &str) {
    let is_permission_error = (asset_err.contains("HTTP 401") || asset_err.contains("HTTP 403"))
        || (server_err.contains("HTTP 401") || server_err.contains("HTTP 403"));

    let (title, subtitle) = if is_permission_error {
        (
            "Missing API Permissions",
            "Your API key must include the 'server.about', 'server.versionCheck', 'server.statistics', and 'asset.statistics' permissions to view stats.",
        )
    } else {
        (
            "Unable to load statistics",
            "Check your connection and try again.",
        )
    };

    let err_row = libadwaita::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .subtitle_lines(0)
        .build();
    let group = libadwaita::PreferencesGroup::builder().build();
    group.add(&err_row);
    content.append(&group);
}

/// Build a row of colored stat cards arranged in a responsive `FlowBox`.
fn stat_card_row(items: &[(&str, String, &str)]) -> gtk::FlowBox {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .row_spacing(8)
        .column_spacing(8)
        .min_children_per_line(1)
        .max_children_per_line(3)
        .build();

    for (label, value, card_class) in items {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .halign(gtk::Align::Fill)
            .css_classes(vec!["mimick-stat-card".to_string(), card_class.to_string()])
            .build();

        let value_label = gtk::Label::builder()
            .label(value.as_str())
            .halign(gtk::Align::Start)
            .css_classes(vec!["mimick-stat-value".to_string()])
            .build();
        let name_label = gtk::Label::builder()
            .label(*label)
            .halign(gtk::Align::Start)
            .css_classes(vec!["mimick-stat-label".to_string()])
            .build();
        card.append(&value_label);
        card.append(&name_label);
        flow.insert(&card, -1);
    }

    flow
}

/// Build a row for a single user with quota progress bar if applicable.
fn user_row(user: &crate::api_client::UsageByUser) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::builder()
        .title(&user.user_name)
        .subtitle(format!(
            "{} photos \u{b7} {} videos \u{b7} {}",
            format_number(user.photos),
            format_number(user.videos),
            format_size(user.usage),
        ))
        .subtitle_lines(2)
        .build();

    if let Some(quota) = user.quota_size_in_bytes
        && quota > 0
    {
        let fraction = (user.usage as f64 / quota as f64).min(1.0);
        let bar = gtk::ProgressBar::builder()
            .fraction(fraction)
            .valign(gtk::Align::Center)
            .tooltip_text(format!(
                "{} of {}",
                format_size(user.usage),
                format_size(quota)
            ))
            .css_classes(vec!["mimick-quota-bar".to_string()])
            .build();
        row.add_suffix(&bar);
    }

    row
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", size, UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_number_groups_thousands() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(42), "42");
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(1_234_567), "1,234,567");
        assert_eq!(format_number(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn format_size_picks_smallest_meaningful_unit() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.00 KiB");
        assert_eq!(format_size(1024 * 1024), "1.00 MiB");
        assert_eq!(format_size(1024_u64.pow(3)), "1.00 GiB");
    }

    #[test]
    fn format_size_stops_at_tib() {
        // 5 * 1 PiB rounds up to multi-thousand TiB, but we cap units at TiB.
        let pib = 1024_u64.pow(5);
        assert!(format_size(pib).ends_with(" TiB"));
    }
}
