use ksni;
use ksni::TrayMethods;
use tokio::sync::watch;

#[derive(Debug)]
pub struct MimickTray {
    /// Sender used to signal the GTK main loop to open the settings window.
    /// Sending `true` triggers the open; the receiver is polled via glib::timeout_add.
    pub settings_tx: watch::Sender<bool>,
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
            }.into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    std::process::exit(0);
                }),
                ..Default::default()
            }.into(),
        ]
    }
}

/// Launch the system tray and return the watch receiver for settings-open signals.
pub async fn build_tray() -> Result<(ksni::Handle<MimickTray>, watch::Receiver<bool>), ksni::Error> {
    let (tx, rx) = watch::channel(false);
    let handle = MimickTray { settings_tx: tx }.spawn().await?;
    Ok((handle, rx))
}
