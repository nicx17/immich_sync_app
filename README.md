
# Mimick for Linux

<div align="center">

<img src="src/assets/icon.png" width="128" alt="Mimick Logo">

![Status](https://img.shields.io/badge/Status-beta-FF90C3?style=for-the-badge)
![GitHub Release](https://img.shields.io/github/v/release/nicx17/mimick?include_prereleases&style=for-the-badge&logo=github&color=FF90C3)
![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)
![GTK4](https://img.shields.io/badge/GTK4-7DF12B?style=for-the-badge&logo=gtk&logoColor=white)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-green.svg?style=for-the-badge)](https://www.gnu.org/licenses/gpl-3.0)

</div>

A daemon-based synchronization tool for uploading media files from a Linux desktop to an [Immich](https://immich.app/) server.

Mimick monitors local directories (e.g., `~/Pictures`, `~/Videos`) for new files and automatically uploads them to your Immich instance. It runs securely as a background service and integrates natively with the GNOME desktop environment via a GTK4 / Libadwaita settings window and a StatusNotifier system tray icon.

> [!NOTE]
> **This project is in BETA.** Core features are stable and tested. Please report any issues or edge cases you encounter.

**Status:** Beta. Supports Immich v1.118+.

## Screenshots

| Settings Window | System Tray Menu |
| :---: | :---: |
| ![Settings Window](docs/screenshots/settings_window.png) | ![System Tray Menu](docs/screenshots/tray_icon_screenshot.png) |
| **Ping Test Dialog** | **About Dialog** |
| ![Ping Test Dialog](docs/screenshots/ping_test_screenshot.png) | ![About Dialog](docs/screenshots/about_dialog.png) |

## Features

- **File Monitoring**: Uses `inotify` to detect new files and waits for stable size before uploading.
- **SHA-1 Checksumming**: Deduplication via checksum before upload — exact same logic as the Immich mobile apps.
- **Concurrent Uploads**: 10 parallel worker tasks stream files directly from disk, keeping RAM usage constant.
- **Offline Reliability**: Failed uploads are persisted to `~/.cache/mimick/retries.json` and replayed automatically on next launch.
- **Connectivity**: Automatically switches between **Internal (LAN)** and **External (WAN)** URLs based on availability. At least one must be enabled (enforced by the UI).
- **Custom Album Mapping**: Select an existing remote album, type a custom name, or let the app create an album from the local folder name (e.g., `~/Pictures/Vacation 2024` → Album `Vacation 2024`).
- **One-Way Sync**: Uploads media without modifying local files.
- **Security**: API Key stored in the system keyring via `secret-tool` (libsecret).
- **Desktop Integration**:
  - GTK4 / Libadwaita settings UI (dark mode by default).
  - StatusNotifierItem system tray icon (requires AppIndicator support on GNOME).

---

## Installation (Recommended)

The easiest and official way to install Mimick on any Linux distribution is via our Flatpak repository. This ensures you receive automatic updates whenever a new version is released.

Run these commands in your terminal:


# 1. Add the official Mimick repository

```bash
flatpak remote-add --user --if-not-exists mimick-repo [https://nicx17.github.io/mimick/mimick.flatpakrepo](https://nicx17.github.io/mimick/mimick.flatpakrepo)
```

# 2. Install the application

```bash
flatpak install --user mimick-repo io.github.nicx17.mimick
```
---

## Usage & Configuration

### First Launch

Launch Mimick from your Application Launcher. The settings window opens automatically on first launch.

1. **Internal URL** — LAN address (e.g., `http://192.168.1.50:2283`).
2. **External URL** — WAN/Public address (e.g., `https://photos.example.com`). *At least one must be enabled.*
3. **API Key** — Generate in Immich Web UI under Account Settings > API Keys. Needs **Asset** and **Album** read/create permissions.
4. **Watch Paths** — Add folders to monitor. Each folder can be assigned a target Immich album.

### Autostart

If you want Mimick to start upon login, ensure you have configured it in your desktop environment's startup applications (e.g., GNOME Tweaks), or copy the `.desktop` file to `~/.config/autostart/`.

---

## Building from Source (For Developers)

If you prefer to compile Mimick yourself, you can build it natively or package it as a local Flatpak.

### Prerequisites (Native Build)

* Rust toolchain (`cargo`): https://rustup.rs
* GTK4 + Libadwaita development headers

**Ubuntu / Debian:**

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-dev pkg-config build-essential libsecret-1-dev

```

**Fedora:**

```bash
sudo dnf install gtk4-devel libadwaita-devel libsecret-devel pkg-config

```

**Arch Linux:**

```bash
sudo pacman -S gtk4 libadwaita libsecret pkgconf base-devel

```

### Native Rust Build

```bash
git clone [https://github.com/nicx17/mimick.git](https://github.com/nicx17/mimick.git)
cd mimick
cargo build --release
# Copy the desktop file and icons from setup/ to ~/.local/share/applications and ~/.local/share/icons for launcher integration

# Run Directly
cargo run                   # start in background mode
cargo run -- --settings     # open the settings window immediately

```

### Local Flatpak Build

```bash
git clone [https://github.com/nicx17/mimick.git](https://github.com/nicx17/mimick.git)
cd mimick
flatpak-builder --user --install --force-clean build-dir io.github.nicx17.mimick.yml
flatpak run io.github.nicx17.mimick

```

---

## Documentation
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [User Guide](docs/USER_GUIDE.md)

## Contributing

Pull requests are welcome. See `CONTRIBUTING.md` for commit and style guidelines.

## Acknowledgments

* Application icon illustration by [Round Icons](https://unsplash.com/@roundicons) on Unsplash.

## License

GNU General Public License v3.0 — see [LICENSE](https://github.com/nicx17/mimick/blob/main/LICENSE).
