# Immich Auto-Sync for Linux: Feature Roadmap & Architecture

## 1. Core Sync Engine (Background Daemon)

- [x] Implement `watchdog` to monitor target directories (`~/Pictures/Screenshots`) via the Linux kernel `inotify` subsystem.
- [x] **One-Way Sync:** Upload files to Immich; never delete local files or download from server.
- [x] **File Filtering:** Whitelist common media types (JPG, PNG, HEIC, MP4) and explicitly ignore sidecar files (XMP).
- [x] Add write-completion detection to ensure files are fully saved before reading (optimized with non-blocking threads).
- [x] **Concurrency Control:** Implement a Worker Queue to process bulk file drops.
- [x] **Parallel Uploads:** Use multi-threaded workers (10 threads) for high-speed batch uploading.
- [x] Create a local retry queue (SQLite or JSON) for offline support.
- [x] **Reverse Proxy Error Handling:** Catch HTTP 413, 502, 504.
- [x] Implement daemon logging to standard Linux locations (`journald`).

## 2. Immich API Client & Network Routing

- [x] **Smart URL Routing (LAN vs. WAN):** Implement a lightweight ping to the Internal URL. Fallback to External URL if unreachable.
- [x] **Pre-Upload Deduplication:** Calculate SHA-1 checksum locally; verify against API before uploading (via 409 Conflict).
- [x] **Resiliency:** Implement exponential backoff for failed uploads and connection pooling.
- [x] Handle authentication via `x-api-key` headers.
- [x] Construct the `multipart/form-data` payload.
- [x] **Strict Metadata Formatting:** Format timestamps strictly as ISO 8601 UTC.
- [x] **Smart Albums:** Automatically create albums based on local folder names and add uploads to them.

## 3. Configuration & Security

- [x] Read/write settings to standard XDG directories (`~/.config/immich-sync/config.json`).
- [x] Integrate Python `keyring` (Secret Service API via DBus) for secure API key storage.
- [x] Support watching multiple directories simultaneously.

## 4. System Tray Interface (Anchor UI)

- [x] Implement `pystray` (AppIndicator/StatusNotifierItem protocols).
- [x] Add dynamic icon states and a context menu (Pause, Sync Now, Settings, Quit).
- [x] **Wayland Support:** Force AppIndicator backend via environment variables for GNOME/KDE Wayland.

## 5. Settings Window (Configurator UI)

- [x] Build a lightweight GUI window (PySide6).
- [x] **Dual URL Configuration:** Inputs for Internal and External URLs.
- [x] "Test Connection" button with detailed LAN/WAN reporting.
- [x] **Progress Indication:** Added progress bar for uploads in the UI as well as the notification.

## 6. Desktop Integration

- [x] Write a `systemd` user service file (`immich-sync.service`) for auto-start.
- [x] Implement native desktop notifications (via `dbus` / `libnotify`). Includes progress bar for uploads.

## 7. Packaging & Distribution (New)

- [x] Write a `setup.py` or `pyproject.toml` for standard Python packaging.
- [x] Create an Arch Linux `PKGBUILD` for submission to the AUR.
- [x] Create an official standalone custom AppImage for self-contained PySide6 distribution directly from GitHub.
- [ ] (Optional) Create a Flatpak manifest for universal distro compatibility.

---
