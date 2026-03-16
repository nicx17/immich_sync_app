# Mimick Wiki

Welcome to the Mimick project wiki.

Mimick is a Linux background app that watches selected folders and syncs photos and videos to an Immich server. It supports native Linux installs and Flatpak, uses a GTK4/Libadwaita settings window, and keeps syncing reliable with retries, startup catch-up scans, and duplicate-aware uploads.

## Start Here

- [Installation](Installation)
- [Configuration and First Run](Configuration-and-First-Run)
- [Sync Behavior](Sync-Behavior)
- [Flatpak and Permissions](Flatpak-and-Permissions)
- [Troubleshooting](Troubleshooting)

## Developer Docs

- [Architecture](Architecture)
- [Development](Development)
- [Testing](Testing)
- [Release Operations](Release-Operations)

## Project Notes

- Mimick is a one-way sync tool. It uploads local media to Immich and does not modify local files.
- The app stores API keys in the desktop keyring and keeps operational state in `~/.cache/mimick/`.
- Startup rescans use a local sync index so already-synced unchanged files are skipped quickly.
