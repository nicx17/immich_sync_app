# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

### Added
- Expanded supported media formats to match latest Immich server: AVIF, BMP, HEIF, JPEG 2000, JPEG XL, PSD, SVG, 3GPP, AVI, FLV, M4V, Matroska (MKV), MP2T, MXF, and more. The app now recognizes and uploads all Immich-compatible image and video extensions.


## [9.0.0] - 2026-03-29

### Fixed
- Fixed an Immich asset timestamp regression where newly uploaded files could land at the wrong timeline time or lose their intended timezone after server-side metadata processing.

### Changed
- Upload metadata handling now preserves filesystem-based creation times more reliably and reapplies the local timezone after upload so Immich keeps the correct asset date placement.
- The settings window now uses `Status` and `Settings` pages, shows the first-run API-key guidance at the top of the configuration flow, and no longer forces dark mode.
- `Save & Restart` has been replaced with live `Save Changes` behavior that updates the running API client, queue policy, upload worker count, and watched folders without relaunching Mimick.
- Watch-folder changes now reconfigure the live filesystem monitor in place, so adding or removing folders takes effect immediately after saving.


## [8.0.0] - 2026-03-25

### Added
- **Health Dashboard**: A visual status area on the Controls page showing active server route, watched folder count, pending items, recent retries, and latest errors.
- **Per-Folder Status**: The settings UI now displays the pending queue count and last sync time specific to each configured watch folder.
- **Permission Health Checks**: On startup, Mimick now verifies that it still has read access to all configured directories. If a Flatpak permission is lost, a warning is prominently displayed.
- **Safer Startup Catch-Up Controls**: Added a "Startup Catch-up Mode" dropdown in settings allowing users to limit startup scans to "Recent Changed Only (7 days)" or "New Files Only" to save on disk I/O.
- **Actionable Errors**: Meaningful connection failure and folder access loss messages replace generic request timeouts.
- **Better Album Picker**: The per-folder album selector is now a modal search dialog. Users can filter existing Immich albums by name, pick the default folder-name behavior, or type a new name to create an album on the fly.
- **First-Run Wizard**: When no API key is configured, Mimick automatically opens the Setup page and displays a welcome banner. The "Save & Restart" button is disabled until an API key is entered, preventing silent broken-connection states.
- **Notifications That Matter**: Replaced per-file notification spam with a single batch summary notification that fires once a sync cycle completes. Added a dedicated "Connection Lost" notification that fires after consecutive failures.
- **Upload Concurrency**: Users can now configure between 1 and 10 parallel upload workers in the settings, allowing for better tuning based on network capacity.
- **Quiet Hours**: Added a configurable quiet-hours window to pause uploads during specific hours of the day (e.g., to prevent impact on nighttime network usage).
- **Mobile Responsive UI**: Refactored the settings window from a rigid `adw::ApplicationWindow` to a native `adw::PreferencesWindow`. Primary controls and action buttons now use adaptive `FlowBox` layouts that auto-stack vertically on narrow displays (down to 360px), ensuring the app is fully usable on Linux phones and small monitors.
- **Adaptive Folder Rows**: Watch folder entries now use `adw::ExpanderRow` to hide additional settings (Album, Rules, Remove) until clicked, maximizing screen space on mobile.

### Fixed
- Fixed an "endless loop" bug where offline network conditions caused already-synced files to be incorrectly re-queued for reassociation.
- Fixed an issue where the processed file count in the UI would increment infinitely during network failures.
- Fixed a bug where a previously selected album target reverted visually to a "Custom Album" field after an application restart.

## [7.0.0] - 2026-03-22

### Added
- A queue inspector in the settings window with recent queue activity, failed-item visibility, per-item retry actions, `Retry All Failed`, and `Clear Failed Queue`.
- Manual sync controls in both the settings window and tray menu with `Pause / Resume` and `Sync Now` actions.
- Per-folder sync rules for ignoring hidden files, limiting maximum file size, and restricting allowed file extensions.
- A diagnostics export bundle that writes a support-friendly snapshot containing a summary, config copy, status cache, retry queue, sync index, and log file without including the API key.
- Best-effort environment-aware pausing for metered-network and battery-power operation.

### Changed
- Startup scans and live monitoring now apply the same per-folder rule checks and temporary-file filtering before queueing uploads.
- Shared runtime state now records recent queue events, pause reasons, the last completed file, and diagnostics export counts for better visibility and supportability.
- The settings window now separates `Setup` and `Controls`, uses a slimmer layout, and keeps `Close`, `Quit`, and `Save & Restart` pinned in a footer.
- Documentation now covers the new sync controls, diagnostics workflow, per-folder rules, and current test/packaging flow.
- CI and Flatpak publishing documentation now match the current `cargo fmt`, `cargo clippy --locked`, `cargo test --locked`, and containerized Flatpak build setup.

## [6.0.0] - 2026-03-15

### Added
- A startup catch-up scan that walks watched folders on launch and queues media that was missed while Mimick was not running.
- A local sync index that records previously synced files so unchanged media can be skipped quickly on later startups.

### Changed
- Changing the target Immich album for a watched folder now causes unchanged files to be reassociated to the new album on a later startup instead of being ignored.
- If a previously targeted album no longer exists, Mimick now refreshes album resolution and retries with the current configured album target.
- Terminal and file logs now include timestamped detailed formatting for easier troubleshooting.
- Flatpak tray integration now uses a narrower StatusNotifier permission model and no longer requests broad `org.kde.*` bus-name ownership.

## [5.0.1] - 2026-03-14

### Added
- GitHub releases now attach a signed `mimick.flatpakrepo` file and a `SHA256SUMS.txt` checksum file for easier end-user installs.

### Changed
- The GitHub Pages Flatpak repository workflow now signs published repo metadata with a dedicated GPG key and embeds the public key in the generated `.flatpakrepo` file.
- The release workflow now uses the same Flatpak signing key material from GitHub Actions secrets so release assets match the published repository trust chain.

## [5.0.0] - 2026-03-14

### Added
- A built-in **Run on Startup** setting that requests desktop-portal background permission in Flatpak builds and writes a native autostart desktop entry outside Flatpak.
- Friendly folder labels for portal-backed watch directories, so selected Flatpak folders show names like `Screenshots` instead of raw `/run/user/.../doc/...` paths.
- Real **Save & Restart** behavior that relaunches Mimick after settings are saved.
- Explicit **Close** and **Quit** actions in the settings window, plus a launcher **Quit Mimick** desktop action.
- A published GitHub Pages landing page for the Flatpak repository with direct install instructions and repository links.

### Changed
- Flatpak builds now use selected-folder access through the file chooser portal instead of `--filesystem=home`.
- Folder monitoring inside Flatpak now uses a polling watcher so portal-backed directories continue to sync reliably.
- Local Flatpak development builds now use the same selected-folder permission model as the deployed app.
- App quit paths now shut down gracefully instead of using a hard process exit from the tray.
- The Flatpak repository landing page has been redesigned with a simpler, more cohesive visual style and a one-click copy action for install commands.

## [4.0.0] - 2026-03-14

### Changed
- Added Flatpak packaging support
- Removed default photo watch path configuration on startup
- Polished AppStream metadata for Flathub compliance

## [3.0.0] - 2026-03-09

### Added
- **Complete Rust Port**: Entire application rewritten from Python/PySide6 to Rust + GTK4 + Libadwaita. Binary drops from ~80MB (PyInstaller bundle) to ~2MB.
- **Tokio async runtime**: Concurrent upload workers (configurable, default 3) with streaming `reqwest` multipart — constant RAM regardless of file size.
- **In-memory shared state**: `Arc<Mutex<AppState>>` replaces disk-based IPC polling. No disk I/O during normal operation.
- **`flexi_logger`**: Logs written to both stdout (systemd) and `~/.cache/mimick/mimick.log` for persistent debugging.
- **Tray via `ksni`**: StatusNotifierItem tray using a `tokio::sync::watch` channel — no zombie processes, no D-Bus spawn.
- **Duplicate upload prevention**: `active_tasks` HashSet in the file monitor prevents multiple `wait_for_file_completion` tasks for the same file during long writes (e.g. screencasts).
- **App ID standardized**: Unified to `io.github.nicx17.mimick` across the binary, `.desktop`, `.metainfo.xml`, icons, and install scripts.
- **AppImage packaging**: `build_test_appimage.sh` compiles a release binary and assembles a standard AppDir in 5 steps.

### Changed
- Settings window uses hide-on-close (built once per process) — eliminates repeated GTK widget tree allocations.
- `ImmichApiClient` is a singleton (`OnceLock`) — single `reqwest` connection pool for the lifetime of the process.
- Autostart now uses `io.github.nicx17.mimick.desktop` symlink.
- All documentation (`ARCHITECTURE.md`, `DEVELOPMENT.md`, `TESTING.md`, `TROUBLESHOOTING.md`, `APPIMAGE_CREATION.md`) updated for Rust/Cargo.
- GitHub Actions release workflow updated for Rust toolchain.
- CodeQL analysis updated to use `languages: rust` with `build-mode: none`.

### Removed
- All Python source files (`main.py`, `settings_window.py`, `tray_icon.py`, etc.)
- `requirements.txt`, `pyproject.toml`, `setup.py`, `MANIFEST.in`

## [2.0.1] - 2026-03-08

### Changed
- Renamed repository and backend strings from `immich_sync_app` to `mimick`

## [2.0.0] - 2026-03-08

### Added
- **Complete Rebranding to Mimick**: Officially renamed the project from "Immich Sync" to "Mimick" to establish a unique identity and drop the generic moniker. All internal app IDs, metadata, documentations, and daemon variables have been fully synchronized.
- **GTK4 / libadwaita Migration**: Totally replaced the heavy PySide6 UI framework with a native, responsive GTK4 + libadwaita interface. The application now perfectly mimics the native look and feel of modern GNOME and KDE desktop environments.
- **Scalable Vector Icons**: Modernized app icon integration by deploying the high-resolution `mimick.svg` into system `hicolor/scalable/apps/` directories.

### Changed
- AppImage build scripts and installation loops have been completely restructured to support the new `mimick` nomenclature and GTK requirements.
- Standardized the GNOME window `StartupWMClass` bindings effectively preventing stray or duplicate launcher icons on Wayland/X11 desktops.

## [1.0.2] - 2026-03-07

### Fixed
- **AppImage Python 3.12 Bundle**: Overhauled AppImage scripts to download and bundle a standalone `python-build-standalone` payload, resolving missing C-Extension (`Pillow`) bugs on modern OS hosts (like Ubuntu 24).
- **GTK AppIndicator Native Support**: Added `PyGObject` to the packaged environment and successfully bridged host GUI drivers via `GI_TYPELIB_PATH` to ensure system tray icon features don't crash under isolated packaging.
- **Duplicate Album Creation Race Condition**: Implemented `threading.Lock()` on the `get_or_create_album` REST endpoint to ensure multiple simultaneous workers handling bulk image drops to new directories don't spawn multiple identical albums on the server if they bypass the cache at the same time.
- **Ubuntu 24 Tray Icon Crash**: Added graceful try/except block wrapping around the `TrayIcon/pystray` initialization loop. On modern Desktop Environments (Ubuntu 24 Wayland / Mutter) that deny AppIndicator injection, the application no longer permanently fails. Instead it safely disables the visual tray while dropping seamlessly into a headless background daemon. Launching from the GUI menu with the tray disabled intelligently loads the Settings Window.

## [1.0.1] - 2026-03-07

### Added
- **File Move/Rename Support**: `ImmichEventHandler` now captures `on_moved` watchdog events. Temporary file downloads (e.g. `video.mp4.tmp` from web browsers, rsync, Syncthing) that later rename internally to a valid media extension are now successfully captured and pushed to the upload queue.

### Fixed
- **Incomplete Video File Upload Bug (`wait_for_file_completion`)**: Prevented massive media files (like 30-minute GUI screencasts) from triggering early timeouts before they were fully written. Replaced absolute 10s wait logic with an adaptive 300-second *idle* timeout loop; continuously growing items dynamically rest the counter keeping uploads safe regardless of copy duration.


## [1.0.0] - 2026-03-06

### Added
- **Animated UI Toggles**: Added custom beautiful `SlideSwitch` CSS animations to the Settings Window allowing users to visually toggle Internal (LAN) vs External (WAN) URL behaviors on and off.
- Config now persists `internal_url_enabled` and `external_url_enabled` booleans.
- Expanded testing coverage for `api_client` and `config` including advanced error-state simulation and file-system failure catching.

### Fixed
- **Captive Portal Bug Fix**: The API Ping routing logic now strictly requires a `{"res": "pong"}` JSON payload resolution to avoid falsely pinging local cafe Wi-Fi captive portals and breaking sync loops.
- **Failover Cache Reset Bug Fix**: Fixed an issue where a timeout connection to the Internal URL loop would not flush the active API endpoint causing the logic to effectively loop blindly instead of bouncing sequentially to the External URL.
- Fixed critical App UI freezing (App Not Responding) during testing connection pings syncing via a synchronous socket process - now visually wraps tests via Qt override wait cursors.
- **Queue Offline Resolution Fix**: Fixed a data-loss bug that permanently flushed queued failed uploads if the user closed the window. Implemented `~/.cache/mimick/retries.json` to seamlessly save pending cache limits, accompanied by an explicit background locking worker loop restoring files successfully.

## [0.2.0] - 2026-03-06

### Added


- AppImage distribution! A new fully packaged AppImage version of `mimick` is now available, bundling `PySide6` and all Python dependencies into a single, highly portable executable.
- Introduced `AI_CONTEXT.md` to help agentic tools understand the application's unique multi-threaded API architecture, system constraints, and X11/Wayland workarounds.

### Fixed


- Fixed critical Qt 6 Wayland connection error where the DBus portal rejected window launching (`Could not register app ID`). Application metadata is now strictly set before Qt engine initialization.
- Fixed a metadata warning regarding the `.desktop` suffix in Qt's `setDesktopFileName` handler.
- Fixed buggy AppRun bash script backslash escaping that was causing `Exec format error` exceptions inside generated `AppImage` distributions.
- Fixed a bug where native AppImages were trying to execute `main.py` outside of isolated module logic.

### Changed

- Promoted project status from Alpha to properly release `v0.2.0` (removed beta tags completely from code structure and internal About tags).
- Modified API `_ping` function tests from testing generic text formats to raw JSON validation checks.
- Added robust direct-file editing scripts to fully automate AppImage extraction, generation, and packaging (`build_test_appimage.sh`).
- Updated PySide6 dependencies and application system documentation (`ARCHITECTURE.md` and `DEVELOPMENT.md`).

## [0.1.0-alpha] - 2026-03-03

### Added

- Created `AppImage` deployment script and comprehensive guide for easy Linux distribution natively bundling `PySide6` and python libraries.
- Extended testing suite to cover `notifications`, `tray_icon`, and `state_manager` using fully mocked implementations.
- Implemented desktop entry integration and `install.sh` enhancements standardizing icons to `/usr/share/pixmaps`.
- Added new AppImage-specific helper scripts (`install-appimage.sh` and `uninstall-appimage.sh`).
- Added User Guide (`docs/USER_GUIDE.md`), Testing Guide (`docs/TESTING.md`), and Architecture Guide (`docs/ARCHITECTURE.md`) to assist end-users and developers.
- Added `CONTRIBUTING.md` and initial project scaffolding.
- Added modern structural badges and active Alpha-phase developmental warnings to the `README.md`.
- Properly credited application icon to Unsplash's Round Icons.

### Fixed

- Fixed issue on GNOME/X11 where the application icon would not render in the dock or settings window due to misaligned `.desktop` metadata (`StartupWMClass`).
- Revised the `install.sh` routine to ensure Python virtual environment integrity and `pip` availability before attempting dependency installation.

### Changed

- Transitioned project license from MIT to **GPL-3.0**.
- Refactored PySide6 window initializations to fallback to a reliable absolute image path as opposed to breaking natively on XDG theme engines lacking caching.
- Updated `pyproject.toml` and `setup.py` metadata for publishing (PyPI readiness), adding GPLv3 and Alpha classifiers.
