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

## 8. GTK4 / Libadwaita Architecture Overhaul (Future Phase)

- [ ] Migrate `PySide6` UI (`settings_window.py`) to `PyGObject` (`gi.repository.Gtk` / `Adw`).
- [ ] Implement `Gtk.Application` to take advantage of native D-Bus application process locking (deprecating custom Unix Sockets & `QLocalServer`).
- [ ] Redesign settings UI from raw data grids (`QTableWidget`) to native GNOME `Adw.PreferencesPage` and `Adw.ActionRow` layouts.
- [ ] Separate the System Tray (`pystray`) into an isolated thread decoupled from the GTK main loop since GTK4 no longer natively supports system trays.
- [ ] Build a Flathub-ready Flatpak manifest using GNOME Builder with strict `xdg-desktop-portal` filesystem constraints (`--filesystem=xdg-pictures`).
- [ ] **Memory Optimization (GTK):** Merge the standalone PySide6 UI subprocess and daemon into a unified `Gtk.Application` loop to reduce the memory footprint by utilizing GNOME's pre-loaded shared system libraries (~50MB projected savings).
- [ ] **Memory Optimization (GTK):** Merge the standalone UI subprocess and daemon into a unified `Gtk.Application` to drastically reduce the memory footprint by utilizing GNOME's pre-loaded shared system libraries instead of detached Qt bindings.


## 9. fix Bugs

- [x] **Bug fix:auto switch url to external when internal not available**
- [x] **on preffered local network check** If not on the home network always use external url
- [x] **handle ping to internal gracefully.. diagnose appnot responding behaviour** 
- [x] **do not try to upload to local instacne unless its verified that it exists or reachable by using ping**

- [x] **investigate que behaviour**  missed images to be reuploaded once app running again
- [x] **Analyse api key permissions**
- [x] bug uploads video files befor they have been completely written in the folder being watched.. eg screencapture video screencasts 
- [x] when a new folder is added to be watched and the trarget folder on the remote dosent exist. and multiple files are added to the local folder ... each worker handling each image ends up creating a seperate folder on the immich instance

-[x] tray icon on ubuntu dosent work so allow app to run without that feature.. clicking appicon in menu should not invoke icon tray unless tray icon works on that os config.. settings window should open as usual and background

## 10. complete folder sync
- [ ] **complete folder sync** sync folder added to remote or vice versa.. toggles for config.. sncy remote changes to local and sync local changes to remote
