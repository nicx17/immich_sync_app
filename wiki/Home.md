# Mimick Wiki

Welcome to the Mimick project wiki.

Mimick is a Linux background app that watches selected folders and syncs photos and videos to an Immich server. It supports native Linux installs and Flatpak, uses a GTK4/Libadwaita settings window, and keeps syncing reliable with retries, startup catch-up scans, and duplicate-aware uploads.

<div align="center">
  <img src="https://raw.githubusercontent.com/nicx17/mimick/main/docs/screenshots/photos_page_view_sidebar_on.png" width="80%" alt="Mimick Library View" />
</div>

## Start Here

<div align="center">

[![Installation](https://img.shields.io/badge/Installation-Guide-1F6FEB?style=for-the-badge&labelColor=1F6FEB)](Installation)
[![Configuration](https://img.shields.io/badge/Configuration-First_Run-2E8B57?style=for-the-badge&labelColor=2E8B57)](Configuration-and-First-Run)
[![Library View](https://img.shields.io/badge/Library-View-FF90C3?style=for-the-badge&labelColor=FF90C3)](Library-View-User-Guide)
[![Sync Behavior](https://img.shields.io/badge/Sync-Behavior-B8860B?style=for-the-badge&labelColor=B8860B)](Sync-Behavior)
[![Performance](https://img.shields.io/badge/Performance-Tuning-0E7490?style=for-the-badge&labelColor=0E7490)](Performance-Tuning)
[![Flatpak](https://img.shields.io/badge/Flatpak-Permissions-6366F1?style=for-the-badge&labelColor=6366F1)](Flatpak-and-Permissions)
[![Troubleshooting](https://img.shields.io/badge/Troubleshooting-Help-CB4B16?style=for-the-badge&labelColor=CB4B16)](Troubleshooting)

</div>

## Developers and Contributors

<div align="center">

[![Architecture](https://img.shields.io/badge/Architecture-Overview-444444?style=for-the-badge&labelColor=444444)](Architecture)
[![Development](https://img.shields.io/badge/Development-Guide-444444?style=for-the-badge&labelColor=444444)](Development)
[![Testing](https://img.shields.io/badge/Testing-Guide-444444?style=for-the-badge&labelColor=444444)](Testing)
[![Release Ops](https://img.shields.io/badge/Release-Operations-444444?style=for-the-badge&labelColor=444444)](Release-Operations)
[![Repo Automation](https://img.shields.io/badge/Repository-Automation-444444?style=for-the-badge&labelColor=444444)](Repository-Automation)

</div>

## Project Notes

- Mimick is a one-way sync tool. It uploads local media to Immich and does not modify local files.
- The app stores API keys in the desktop keyring and keeps operational state in `~/.cache/mimick/`.
- Startup rescans use a local sync index so already-synced unchanged files are skipped quickly.
- Flatpak distribution is signed; verify the published fingerprint before trusting a new repo setup.

## Current App Highlights

- Two-page `Settings` / `Status` settings window
- Optional in-app library viewer with albums, Explore, and search
- Queue inspector with retry actions
- Per-folder rules for hidden paths, size limits, and extension filters
- Diagnostics bundle export for support and bug reports
- Metered-network and battery-aware upload deferral
