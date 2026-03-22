# Mimick Wiki

Welcome to the Mimick project wiki.

Mimick is a Linux background app that watches selected folders and syncs photos and videos to an Immich server. It supports native Linux installs and Flatpak, uses a GTK4/Libadwaita settings window, and keeps syncing reliable with retries, startup catch-up scans, and duplicate-aware uploads.

## Start Here

- [Installation](https://github.com/nicx17/mimick/wiki/Installation)
- [Configuration and First Run](https://github.com/nicx17/mimick/wiki/Configuration-and-First-Run)
- [Sync Behavior](https://github.com/nicx17/mimick/wiki/Sync-Behavior)
- [Flatpak and Permissions](https://github.com/nicx17/mimick/wiki/Flatpak-and-Permissions)
- [Troubleshooting](https://github.com/nicx17/mimick/wiki/Troubleshooting)

## Maintainers and Contributors

- [Architecture](https://github.com/nicx17/mimick/wiki/Architecture)
- [Development](https://github.com/nicx17/mimick/wiki/Development)
- [Testing](https://github.com/nicx17/mimick/wiki/Testing)
- [Repository Automation](https://github.com/nicx17/mimick/wiki/Repository-Automation)
- [Release Operations](https://github.com/nicx17/mimick/wiki/Release-Operations)

## Project Notes

- Mimick is a one-way sync tool. It uploads local media to Immich and does not modify local files.
- The app stores API keys in the desktop keyring and keeps operational state in `~/.cache/mimick/`.
- Startup rescans use a local sync index so already-synced unchanged files are skipped quickly.
- Flatpak distribution is signed; verify the published fingerprint before trusting a new repo setup.

## Current App Highlights

- Two-page `Setup` / `Controls` settings window
- Queue inspector with retry actions
- Per-folder rules for hidden paths, size limits, and extension filters
- Diagnostics bundle export for support and bug reports
- Metered-network and battery-aware upload deferral
