# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
