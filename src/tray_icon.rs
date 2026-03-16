//! StatusNotifier tray integration and its GTK-facing control signals.

use ksni;
use ksni::TrayMethods;
use tokio::sync::watch;

/// Tray state shared with the ksni menu callbacks.
#[derive(Debug)]
pub struct MimickTray {
    /// Sender used to signal the GTK main loop to open the settings window.
    /// Sending `true` triggers the open; the receiver is polled via glib::timeout_add.
    pub settings_tx: watch::Sender<bool>,
    /// Sender used to request a graceful application quit from the GTK main loop.
    pub quit_tx: watch::Sender<bool>,
}

impl ksni::Tray for MimickTray {
    fn id(&self) -> String {
        "mimick_tray".to_string()
    }

    fn icon_name(&self) -> String {
        "io.github.nicx17.mimick".to_string()
    }

    fn title(&self) -> String {
        "Mimick Sync".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Settings".into(),
                activate: Box::new(|tray: &mut Self| {
                    // Signal the GTK main loop — no new process spawned.
                    let _ = tray.settings_tx.send(true);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.quit_tx.send(true);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Launch the tray and return receivers for settings-open and quit requests.
pub async fn build_tray() -> Result<
    (
        ksni::Handle<MimickTray>,
        watch::Receiver<bool>,
        watch::Receiver<bool>,
    ),
    ksni::Error,
> {
    let (settings_tx, settings_rx) = watch::channel(false);
    let (quit_tx, quit_rx) = watch::channel(false);
    let tray = MimickTray {
        settings_tx,
        quit_tx,
    };
    let handle = if ashpd::is_sandboxed() {
        // Flatpak sessions already broker the item through the watcher, so we avoid owning
        // an additional D-Bus name and keep the permission request narrower.
        tray.disable_dbus_name(true).spawn().await?
    } else {
        tray.spawn().await?
    };
    Ok((handle, settings_rx, quit_rx))
}
