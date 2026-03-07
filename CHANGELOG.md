# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- **Queue Offline Resolution Fix**: Fixed a data-loss bug that permanently flushed queued failed uploads if the user closed the window. Implemented `~/.cache/immich-sync/retries.json` to seamlessly save pending cache limits, accompanied by an explicit background locking worker loop restoring files successfully.

## [0.2.0] - 2026-03-06

### Added


- AppImage distribution! A new fully packaged AppImage version of `immich-sync` is now available, bundling `PySide6` and all Python dependencies into a single, highly portable executable.
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
