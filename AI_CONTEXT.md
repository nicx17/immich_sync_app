# Mimick Application Context

## Application Overview

`mimick` is a multi-threaded Linux desktop daemon and settings UI written in **Rust**, designed to automatically synchronize local media files (photos, videos) with an [Immich](https://immich.app/) server.

## Architecture & Core Components

The application runs a single process with a Tokio async runtime for background work and a GTK4 main loop for the settings UI. All modules communicate via `Arc<Mutex<>>` shared state and `tokio::sync::mpsc` channels.

### 1. File Monitoring (`src/monitor.rs`)
- Uses `notify` crate to listen for `inotify` filesystem events (`Create`, `Modify`).
- **Debouncing:** Spawns a thread per new file that polls size at 1-second intervals, requiring 3 consecutive identical reads before considering the file stable (`wait_for_file_completion`).
- **Checksumming:** Calculates SHA-1 in 64KB chunks (`compute_sha1_chunked`) to support Immich's deduplication logic without loading whole files into RAM.

### 2. Queue Manager (`src/queue_manager.rs`)
- Spawns N Tokio worker tasks (default 10) sharing a single `mpsc::Receiver<FileTask>`.
- **Offline Reliability:** Failed uploads are persisted to `~/.cache/mimick/retries.json` via an atomic rename pattern. Retries are re-queued on next launch with deduplication.

### 3. API Client (`src/api_client.rs`)
- Interfaces with the Immich REST API via `reqwest`.
- **Streaming uploads:** Files are streamed via `tokio::fs::File` + `FramedRead` + `reqwest::Body::wrap_stream` — RAM usage is constant regardless of file size.
- Manages album creation and asset-album association (`get_or_create_album`, `add_assets_to_album`).

### 4. Configuration (`src/config.rs`)
- Serializes to `~/.config/mimick/config.json` via `serde_json`.
- **API key security:** Stored and retrieved via `secret-tool` (libsecret). Never written in plain text.
- `WatchPathEntry` is an untagged `serde` enum supporting both plain string paths (legacy) and per-folder album config objects.

### 5. State Manager (`src/state_manager.rs`)
- Writes upload progress to `~/.cache/mimick/status.json` using atomic rename (write `.tmp`, then `rename`).
- Consumed by the GTK UI polling loop to update the progress bar and status label.

### 6. Settings Window (`src/settings_window.rs`)
- Built with `gtk4` + `libadwaita` crates.
- Uses `adw::ApplicationWindow`, `adw::PreferencesGroup`, `adw::ActionRow`.
- Album selection per folder row uses `gtk::DropDown` + `gtk::Entry` (no deprecated `ComboBoxText`).
- Validation dialogs use `gtk::AlertDialog` (GTK 4.10+, no deprecated `MessageDialog`).
- Toggle validation prevents both URL switches being off at the same time.

### 7. System Tray (`src/tray_icon.rs`)
- Built with `ksni` crate (StatusNotifierItem DBus protocol).
- Requires `org.kde.StatusNotifierWatcher` DBus service. On stock GNOME without the AppIndicator extension, this will fail with `Watcher(ServiceUnknown)` — this is expected and harmless.

## Project Structure

- `src/` — Rust source modules.
- `setup/` — Packaging: `mimick.desktop`, `mimick.service` (systemd), `PKGBUILD` (Arch Linux), icons, metainfo.
- `docs/` — Documentation: `USER_GUIDE.md`, `TROUBLESHOOTING.md`, `APPIMAGE_CREATION.md`.
- `install.sh` / `uninstall.sh` — Build-from-source install scripts.
- `build_appimage.sh` — Packages the release binary into an AppImage.
- `pyimplement/` — Archived Python reference implementation (read-only).

## Tech Stack

- **Language:** Rust (edition 2024)
- **Async runtime:** Tokio
- **GUI:** GTK4 + Libadwaita (`gtk4`, `libadwaita` crates)
- **HTTP:** `reqwest` (rustls, streaming, multipart)
- **File watching:** `notify`
- **Tray:** `ksni`
- **Serialization:** `serde` + `serde_json`
- **Checksums:** `sha1`
- **Testing:** built-in `cargo test`, `tempfile` dev-dep for filesystem isolation
- **Packaging:** AppImage, PKGBUILD, `.desktop` / `.service` integration
- **Current Version:** v2.0.1

## Common AI Agent Tasks / Context

- **Concurrency:** All shared state across async tasks uses `Arc<Mutex<>>` or `Arc<tokio::sync::Mutex<>>`. Do not block Tokio threads with `std::sync::Mutex`.
- **GTK thread safety:** GTK objects must only be touched on the main thread. Use `glib::MainContext::channel` or `glib::timeout_add_local` to update UI from async tasks.
- **No deprecated APIs:** All GTK4 widgets must be 4.10+ compliant. Run `cargo clippy` to catch deprecations.
- **Wayland:** The settings window uses `adw::ApplicationWindow` to avoid double titlebars on Wayland/GNOME.
- **API key:** Always use `secret-tool` for keyring access. Never store in config.json.

Use this file as a mental anchor when returning to work on `mimick`.
