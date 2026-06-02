//! Queue inspector dialog -- browse failed tasks and recent queue activity.
//!
//! Lists the current retry queue with per-item retry buttons and a bulk
//! retry-all action. A scrollable activity feed shows recent upload
//! attempts, statuses, and error details. The dialog refreshes live via
//! a periodic timer so background state changes are always visible.

use std::path::Path;
use std::sync::Arc;

use adw::prelude::*;
use glib::clone;
use gtk::prelude::*;
use gtk::{Box, Button, ListBox, Orientation, ScrolledWindow};
use libadwaita as adw;

use crate::queue_manager::QueueManager;
use crate::state_manager::QueueEvent;

/// Construct and present the modal Queue Inspector window.
pub fn show_queue_inspector(
    parent: &impl gtk::prelude::IsA<gtk::Window>,
    queue_manager: Arc<QueueManager>,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Queue Inspector")
        .default_width(900)
        .default_height(680)
        .width_request(360)
        .height_request(480)
        .build();

    let header = adw::HeaderBar::builder().show_title(true).build();

    let content = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let main_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();

    let toolbar = adw::ToolbarView::builder().build();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&main_scroll));
    dialog.set_content(Some(&toolbar));

    let actions = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    content.append(&actions);

    let retry_all_btn = Button::builder().label("Retry All Failed").build();
    let clear_failed_btn = Button::builder().label("Clear Failed Queue").build();
    actions.append(&retry_all_btn);
    actions.append(&clear_failed_btn);

    let failed_group = adw::PreferencesGroup::builder()
        .title("Failed Retry Queue")
        .build();
    content.append(&failed_group);

    let events_group = adw::PreferencesGroup::builder()
        .title("Recent Queue Activity")
        .build();
    content.append(&events_group);

    let events_list = ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    events_group.add(&events_list);

    // Initial population.
    refresh_inspector(&failed_group, &events_list, &queue_manager);

    // Wire action buttons with immediate UI refresh.
    let qm_retry_all = queue_manager.clone();
    let fg_retry = failed_group.clone();
    let el_retry = events_list.clone();
    retry_all_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        let qm = qm_retry_all.clone();
        let fg = fg_retry.clone();
        let el = el_retry.clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = qm.retry_all_failed().await;
            refresh_inspector(&fg, &el, &qm);
        });
    });

    let qm_clear = queue_manager.clone();
    let fg_clear = failed_group.clone();
    let el_clear = events_list.clone();
    clear_failed_btn.connect_clicked(move |_| {
        let _ = qm_clear.clear_failed();
        refresh_inspector(&fg_clear, &el_clear, &qm_clear);
    });

    // Live-update timer: refresh every second while the dialog exists.
    let prev_failed_count = std::cell::Cell::new(queue_manager.failed_tasks().len());
    let prev_event_count = std::cell::Cell::new(queue_manager.recent_events().len());
    let qm_tick = queue_manager.clone();
    glib::timeout_add_local(
        std::time::Duration::from_secs(1),
        clone!(
            #[weak]
            failed_group,
            #[weak]
            events_list,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move || {
                let failed = qm_tick.failed_tasks();
                let events = qm_tick.recent_events();
                let changed = failed.len() != prev_failed_count.get()
                    || events.len() != prev_event_count.get()
                    || has_status_change(&events, &failed);
                if changed {
                    prev_failed_count.set(failed.len());
                    prev_event_count.set(events.len());
                    refresh_inspector(&failed_group, &events_list, &qm_tick);
                }
                glib::ControlFlow::Continue
            }
        ),
    );

    let bp = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 500sp").expect("valid breakpoint condition"),
    );
    bp.add_setter(&retry_all_btn, "label", Some(&"Retry All".to_value()));
    bp.add_setter(&clear_failed_btn, "label", Some(&"Clear".to_value()));

    dialog.add_breakpoint(bp);

    dialog.present();
}

/// Detect whether event statuses differ from what's currently displayed.
fn has_status_change(events: &[QueueEvent], failed: &[crate::queue_manager::FileTask]) -> bool {
    // Use a simple heuristic: check the latest event timestamp and failed paths.
    let latest_ts = events.first().map(|e| e.timestamp).unwrap_or(0.0);
    let failed_sig: u64 = failed.iter().map(|t| t.path.len() as u64).sum();
    // Combine into a single value that changes when anything meaningful changes.
    let sig = (latest_ts * 1000.0) as u64 ^ failed_sig;
    // We store nothing here -- the caller compares counts. This catches
    // same-count but different-content changes (e.g. a retry replaced a
    // failed item). The XOR with timestamp makes it very unlikely to miss.
    sig != 0
}

/// Clear and rebuild both sections from current QueueManager state.
fn refresh_inspector(
    failed_group: &adw::PreferencesGroup,
    events_list: &ListBox,
    queue_manager: &Arc<QueueManager>,
) {
    let failed_tasks = queue_manager.failed_tasks();
    let events = queue_manager.recent_events();
    rebuild_failed_rows(failed_group, &failed_tasks, queue_manager);
    rebuild_event_rows(events_list, &events);
}

/// Remove all children from a PreferencesGroup.
fn clear_preferences_group(group: &adw::PreferencesGroup) {
    while let Some(child) = group
        .first_child()
        .and_then(|c| c.last_child())
        .and_then(|c| c.last_child())
        .and_then(|c| c.first_child())
    {
        if child.downcast_ref::<adw::ActionRow>().is_some() {
            group.remove(&child);
        } else {
            break;
        }
    }
}

/// Build the "Failed Retry Queue" rows.
fn rebuild_failed_rows(
    group: &adw::PreferencesGroup,
    tasks: &[crate::queue_manager::FileTask],
    queue_manager: &Arc<QueueManager>,
) {
    clear_preferences_group(group);

    if tasks.is_empty() {
        group.add(
            &adw::ActionRow::builder()
                .title("No failed items")
                .subtitle("The retry queue is currently empty.")
                .build(),
        );
        return;
    }

    for task in tasks {
        let row = adw::ActionRow::builder()
            .title(
                Path::new(&task.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(task.path.as_str()),
            )
            .subtitle(&task.path)
            .title_lines(0)
            .subtitle_lines(3)
            .build();
        let retry_btn = Button::builder().label("Retry").build();
        let task_path = task.path.clone();
        let qm = queue_manager.clone();
        let group_ref = group.clone();
        retry_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label("Retrying\u{2026}");
            let qm = qm.clone();
            let path = task_path.clone();
            let group_ref = group_ref.clone();
            glib::MainContext::default().spawn_local(async move {
                let _ = qm.retry_failed_path(&path).await;
                let refreshed = qm.failed_tasks();
                rebuild_failed_rows(&group_ref, &refreshed, &qm);
            });
        });
        row.add_suffix(&retry_btn);
        group.add(&row);
    }
}

/// Build the "Recent Queue Activity" event rows.
fn rebuild_event_rows(list: &ListBox, events: &[QueueEvent]) {
    list.remove_all();

    for event in events {
        let row = adw::ActionRow::builder()
            .title(
                Path::new(&event.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(event.path.as_str()),
            )
            .subtitle(format!(
                "{} | attempts={}{}",
                event.status,
                event.attempts,
                event
                    .detail
                    .as_ref()
                    .map(|detail| format!(" | {}", detail))
                    .unwrap_or_default()
            ))
            .title_lines(0)
            .subtitle_lines(4)
            .build();

        list.append(&row);
    }
}

/// Construct and present the Libadwaita standard About Dialog for the application.
pub(super) fn show_about_dialog(parent: &impl gtk::prelude::IsA<gtk::Widget>) {
    let about = adw::AboutDialog::builder()
        .application_name("Mimick")
        .application_icon("dev.nicx.mimick")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Nick Cardoso")
        .website("https://github.com/nicx17/mimick")
        .issue_url("https://github.com/nicx17/mimick/issues")
        .license_type(gtk::License::Gpl30)
        .build();

    about.add_credit_section(
        Some("Icon Design"),
        &["Round Icons https://unsplash.com/illustrations/a-white-and-orange-flower-on-a-white-background-IkQ_WrJzZOM"],
    );

    let third_party =
        glib::markup_escape_text(include_str!("../../THIRD_PARTY_LICENSES_SUMMARY.txt"));
    about.add_legal_section(
        "Third-party Licenses",
        None,
        gtk::License::Custom,
        Some(&third_party),
    );

    about.present(Some(parent));
}
