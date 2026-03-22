# Mimick Wiki

Welcome to the Mimick project wiki.

Mimick is a Linux background app that watches selected folders and syncs photos and videos to an Immich server. It supports native Linux installs and Flatpak, uses a GTK4/Libadwaita settings window, and keeps syncing reliable with retries, startup catch-up scans, and duplicate-aware uploads.

## Start Here

- [Installation](Installation.md)
- [Configuration and First Run](Configuration-and-First-Run.md)
- [Sync Behavior](Sync-Behavior.md)
- [Flatpak and Permissions](Flatpak-and-Permissions.md)
- [Troubleshooting](Troubleshooting.md)

## Maintainers and Contributors

- [Architecture](Architecture.md)
- [Development](Development.md)
- [Testing](Testing.md)
- [Repository Automation](Repository-Automation.md)
- [Release Operations](Release-Operations.md)

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
