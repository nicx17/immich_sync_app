# Mimick Application Context

## Application Overview
`mimick` is a multi-threaded Linux desktop daemon and system tray application designed to automatically synchronize local media files (photos, videos) with an [Immich](https://immich.app/) server.

## Architecture & Core Components
The application is decoupled into a background daemon and a UI process (PySide6-based settings window, GTK-based legacy tray icon).

### 1. File Monitoring (`src/monitor.py`)
- Uses `watchdog` to listen for file system events (`IN_CREATE`, `IN_MOVED_TO`).
- **Debouncing:** Implements checks (`wait_for_file_completion`) to ensure files are fully written before processing. Features adaptive idle-timeouts allowing massive files to copy safely (timeout/size stabilization).
- **Checksumming:** Calculates SHA-1 checksums to support Immich's deduplication logic.

### 2. Queue Manager (`src/queue_manager.py`)
- A thread-safe orchestrator for upload tasks using background worker threads (10 default).
- **Offline Reliability:** Implements a persistent on-disk retry queue (`~/.cache/mimick/retries.json`) with robust file locking and unique thread ID temp files.
- Re-queues failed items securely across daemon restarts to guarantee zero-loss uploads.

### 3. API Client (`src/api_client.py`)
- Interfaces directly with the Immich server's REST API.
- Handles authentication and multipart upload of media assets.
- Manages album creation and adding assets to specific albums (`create_album`, `get_or_create_album`, `add_assets_to_album`).

### 4. UI & Configuration
- **Tray Icon (`src/tray_icon.py`):** Provides a system tray applet using `pystray`. Uses environment variable overrides (`XDG_CURRENT_DESKTOP="Unity"`, `GDK_BACKEND="x11"`) to force a visible AppIndicator/X11 tray icon to avoid Wayland scaling and invisibility bugs.
- **Settings Window (`src/settings_window.py` | `src/settings_main.py`):** Built with PySide6 to manage server URL, API key, and monitored directories. Also displays sync progress polling `status.json`.
- **Storage:** Uses `config.json` for general settings, the System Keyring for secure secrets (API keys), and `status.json` managed by `src/state_manager.py` for syncing state.
- **Notifications (`src/notifications.py`):** Uses standard Linux desktop notifications to alert the user about sync progress or errors.

## Project Structure
- `src/`: Contains all main application source code.
- `tests/`: Extensive unit tests (50+ passing `pytest` cases covering advanced mocks).
- `docs/`: Extensive documentation including architecture, development, troubleshooting, and packaging guides.
- `setup/`: Packaging files including a Desktop entry (`mimick.desktop`), systemd user service (`mimick.service`), and PKGBUILD for Arch Linux packaging.
- `*.AppImage` / `install-appimage.sh`: Logic for building and installing AppImages.

## Tech Stack
- **Language:** Python 3
- **GUI:** PySide6 (Settings) / GTK (Tray Icon depending on backend)
- **Monitoring:** `watchdog`
- **Testing:** `pytest` (heavy use of `mocker` and `requests_mock`)
- **Packaging:** `setuptools`, AppImage, standard Linux `.desktop`/`.service` integration
- **Current Version:** v1.0.2

## Common AI Agent Tasks / Context Usage
When assisting with this repository, an AI agent should keep in mind:
- **Concurrency:** Ensure any changes to the queue or state management respect thread-safety (synchronization locks, background threads).
- **Filesystem Constraints:** When interacting with the file monitor, consider edge cases like large files copying slowly, permission errors, and unsupported formats.
- **UI Decoupling:** The daemon and the Settings window interact primarily through shared config files/keyring state. Changes in one may require reload logic in the other.
- **Wayland Compatibility:** The application has known workarounds for legacy X11 tray icons running under Wayland (`GDK_BACKEND=x11`). Be cautious when modifying `tray_icon.py` or entrypoint logic.
- **Recent Focus:** Resiliency (on-disk queues) and robust file-read-completion strategies for large videos.

Use this file as a mental anchor when returning to work on `mimick`.
