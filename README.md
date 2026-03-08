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

Mimick monitors local directories (e.g., `~/Pictures`, `~/Videos`) for new files and automatically uploads them to your Immich instance. It runs as a background service and integrates with the GNOME desktop environment via a GTK4 / Libadwaita settings window and a StatusNotifier system tray icon.

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

## Installation

### Method 1: AppImage (Recommended)

Download the latest `Mimick-x86_64.AppImage` from the [Releases page](https://github.com/nicx17/mimick/releases), then:

```bash
chmod +x Mimick-*.AppImage
./install-appimage.sh /path/to/Mimick-x86_64.AppImage
```

### Method 2: Build from Source

#### Prerequisites

- Rust toolchain (`cargo`): https://rustup.rs
- GTK4 + Libadwaita development headers

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

#### Build and Install

```bash
git clone https://github.com/nicx17/mimick.git
cd mimick
./install.sh
```

This compiles a release binary and installs it to `~/.local/bin/mimick` along with the desktop entry and icons.

#### Run Directly (Development)

```bash
cargo run                   # start in background mode
cargo run -- --settings     # open the settings window immediately
```

## Usage

### First Launch

Run `mimick` or open it from your Application Launcher. The settings window opens automatically on first launch.

1. **Internal URL** — LAN address (e.g., `http://192.168.1.50:2283`).
2. **External URL** — WAN/Public address (e.g., `https://photos.example.com`).
3. **API Key** — Generate in Immich Web UI under Account Settings > API Keys. Needs **Asset** and **Album** read/create permissions.
4. **Watch Paths** — Add folders to monitor. Each folder can be assigned a target Immich album.

### Autostart

The installer will ask if you want to enable autostart via a `.desktop` symlink in `~/.config/autostart`. Alternatively use systemd:

```bash
mkdir -p ~/.config/systemd/user/
cp setup/mimick.service ~/.config/systemd/user/
# Edit ExecStart to point at ~/.local/bin/mimick
systemctl --user enable --now mimick
```

## Documentation

- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [AppImage Creation](docs/APPIMAGE_CREATION.md)
- [User Guide](docs/USER_GUIDE.md)

## Contributing

Pull requests are welcome. See `CONTRIBUTING.md` for commit and style guidelines.

## Acknowledgments

- Application icon illustration by [Round Icons](https://unsplash.com/@roundicons) on Unsplash.

## License

GNU General Public License v3.0 — see [LICENSE](LICENSE).
