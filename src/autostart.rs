//! Handles autostart integration for both native installations and sandboxed Flatpak builds.

use ashpd::WindowIdentifier;
use ashpd::desktop::background::Background;
use gtk::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

const AUTOSTART_DESKTOP_ID: &str = "io.github.nicx17.mimick.desktop";
const APP_ID: &str = "io.github.nicx17.mimick";
const AUTOSTART_REASON: &str = "Reason for requesting background access: Mimick must run in the background to automatically sync media to Immich.";

pub async fn apply(window: &impl IsA<gtk::Window>, enable: bool) -> Result<bool, String> {
    if enable {
        if is_flatpak_sandbox() {
            request_background_portal(window).await
        } else {
            install_desktop_entry().map(|_| true)
        }
    } else {
        remove_desktop_entry().map(|_| false)
    }
}

fn is_flatpak_sandbox() -> bool {
    Path::new("/.flatpak-info").exists()
}

async fn request_background_portal(window: &impl IsA<gtk::Window>) -> Result<bool, String> {
    let identifier = match window.as_ref().native() {
        Some(native) => WindowIdentifier::from_native(&native).await,
        None => None,
    };

    let response = Background::request()
        .identifier(identifier)
        .reason(AUTOSTART_REASON)
        .auto_start(true)
        .dbus_activatable(false)
        .send()
        .await
        .map_err(|err| format!("Failed to contact the background portal: {err}"))?
        .response()
        .map_err(|err| format!("The desktop rejected the autostart request: {err}"))?;

    Ok(response.auto_start() && response.run_in_background())
}

fn install_desktop_entry() -> Result<(), String> {
    let entry_path = default_autostart_entry_path()?;
    if let Some(parent) = entry_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create autostart directory: {err}"))?;
    }

    let executable = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve the Mimick executable path: {err}"))?;
    let escaped_exec = escape_desktop_exec_arg(&executable.to_string_lossy());

    let desktop_entry = format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Mimick\nComment=Automatically upload photos to Immich\nExec={escaped_exec}\nIcon=io.github.nicx17.mimick\nTerminal=false\nCategories=Utility;\nX-GNOME-Autostart-enabled=true\nStartupNotify=false\n"
    );

    fs::write(&entry_path, desktop_entry)
        .map_err(|err| format!("Failed to write autostart entry: {err}"))?;

    Ok(())
}

fn remove_desktop_entry() -> Result<(), String> {
    for entry_path in autostart_entry_paths()? {
        if entry_path.exists() {
            fs::remove_file(&entry_path)
                .map_err(|err| format!("Failed to remove autostart entry: {err}"))?;
        }
    }
    Ok(())
}

fn default_autostart_entry_path() -> Result<PathBuf, String> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "Could not locate the user config directory.".to_string())?;
    Ok(config_dir.join("autostart").join(AUTOSTART_DESKTOP_ID))
}

fn autostart_entry_paths() -> Result<Vec<PathBuf>, String> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "Could not locate the user config directory.".to_string())?;
    let mut paths = vec![config_dir.join("autostart").join(AUTOSTART_DESKTOP_ID)];

    if let Some(host_config_dir) = flatpak_host_config_dir_from(&config_dir) {
        paths.push(host_config_dir.join("autostart").join(AUTOSTART_DESKTOP_ID));
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Map a sandboxed `~/.var/app/<app-id>/config` path back to the host `~/.config` path.
fn flatpak_host_config_dir_from(config_dir: &Path) -> Option<PathBuf> {
    let app_dir = config_dir.parent()?;
    let app_parent = app_dir.parent()?;
    let var_dir = app_parent.parent()?;
    let host_home = var_dir.parent()?;

    if config_dir.file_name()? != "config" {
        return None;
    }
    if app_dir.file_name()? != APP_ID {
        return None;
    }
    if app_parent.file_name()? != "app" {
        return None;
    }
    if var_dir.file_name()? != ".var" {
        return None;
    }

    Some(host_home.join(".config"))
}

/// Escape an executable path for the `Exec=` field of a desktop entry.
fn escape_desktop_exec_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            ' ' => escaped.push_str("\\ "),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '"' => escaped.push_str("\\\""),
            '\'' => escaped.push_str("\\'"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{escape_desktop_exec_arg, flatpak_host_config_dir_from};
    use std::path::Path;

    #[test]
    fn test_escape_desktop_exec_arg() {
        assert_eq!(
            escape_desktop_exec_arg("/tmp/My App/mimick"),
            "/tmp/My\\ App/mimick"
        );
    }

    #[test]
    fn test_flatpak_host_config_dir_from_sandbox_path() {
        assert_eq!(
            flatpak_host_config_dir_from(Path::new(
                "/home/nick/.var/app/io.github.nicx17.mimick/config"
            ))
            .unwrap(),
            Path::new("/home/nick/.config")
        );
    }
}
