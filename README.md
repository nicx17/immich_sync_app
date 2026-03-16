
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

- **File Monitoring**: Watches selected folders for new files and waits for stable size before uploading.
- **SHA-1 Checksumming**: Deduplication via checksum before upload — exact same logic as the Immich mobile apps.
- **Concurrent Uploads**: 10 parallel worker tasks stream files directly from disk, keeping RAM usage constant.
- **Offline Reliability**: Failed uploads are persisted to `~/.cache/mimick/retries.json` and replayed automatically on next launch.
- **Connectivity**: Automatically switches between **Internal (LAN)** and **External (WAN)** URLs based on availability. At least one must be enabled (enforced by the UI).
- **Custom Album Mapping**: Select an existing remote album, type a custom name, or let the app create an album from the local folder name (e.g., `~/Pictures/Vacation 2024` → Album `Vacation 2024`).
- **One-Way Sync**: Uploads media without modifying local files.
- **Security**: API Key stored in the system keyring via `secret-tool` (libsecret).
- **Autostart**: Optional login startup with desktop-portal permission inside Flatpak and native autostart integration outside Flatpak.
- **Startup Catch-Up**: On launch, Mimick scans watched folders for media that has not been synced yet and queues only new, changed, or retargeted files.
- **Clear Window Controls**: `Close` hides the settings window, while `Quit` stops the app completely.
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

### Verify the Flatpak Repo Key

The published Flatpak repository embeds this signing-key fingerprint:

`04E2 9556 E951 B2EA 15D3 A8EE 632E 1BC5 D956 579C`

You can inspect the currently published key with:

```bash
curl -fsSL https://nicx17.github.io/mimick/mimick.flatpakrepo \
  | sed -n 's/^GPGKey=//p' \
  | base64 -d > /tmp/mimick-repo-public.gpg

gpg --show-keys --fingerprint /tmp/mimick-repo-public.gpg
```

Compare the printed fingerprint to the value above. The email address alone is not the trust anchor; the fingerprint is.
---

## Usage & Configuration

### First Launch

Launch Mimick from your Application Launcher. The settings window opens automatically on first launch.

1. **Internal URL** — LAN address (e.g., `http://192.168.1.50:2283`).
2. **External URL** — WAN/Public address (e.g., `https://photos.example.com`). *At least one must be enabled.*
3. **API Key** — Generate in Immich Web UI under Account Settings > API Keys. Needs **Asset** and **Album** read/create permissions.
4. **Watch Paths** — Add folders to monitor with the built-in folder picker. Each folder can be assigned a target Immich album.
5. **Run on Startup** — Enable this in the **Behavior** section to start Mimick automatically when you log in.
6. **Save & Restart** — Applies your settings and relaunches Mimick automatically.
7. **Close / Quit** — `Close` hides the settings window and leaves Mimick running; `Quit` fully exits the app.

### Autostart

Use the built-in **Run on Startup** switch in the settings window.

* Flatpak builds request background/autostart permission through the desktop portal.
* Native builds write an autostart desktop entry to `~/.config/autostart/io.github.nicx17.mimick.desktop`.

### Folder Access

Mimick now uses selected-folder access instead of full home-directory access in Flatpak.

* Add watch folders from the settings window so the file chooser portal can grant access.
* If you are upgrading from an older build that had full home access, re-add your existing watch folders once so the new permission model can take effect.
* Portal-backed folders may appear by name in the UI and logs instead of showing the raw `/run/user/.../doc/...` sandbox path.

### Existing Files and Album Changes

Mimick does not only sync files created while it is already running.

* On startup, Mimick rescans watched folders and queues media that has not been synced yet.
* A local sync index is used so unchanged files that are already known to be synced are skipped quickly.
* If you change the target Immich album for a watched folder, unchanged files can be reassociated to the new album on the next startup without forcing a full reupload.
* If a previously targeted album was deleted, Mimick refreshes album resolution and recreates or rebinds the target album as needed.

### Quitting vs Closing

Mimick is a background app, so closing the settings window does not quit it.

* Use **Close** in the settings window or the window close button to hide the window and keep Mimick running in the tray.
* Use **Quit** from the tray menu, the settings window, or the launcher action to stop the app completely.

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

Logs written to the terminal and to `~/.cache/mimick/mimick.log` now include timestamps.

### Local Flatpak Build

```bash
git clone [https://github.com/nicx17/mimick.git](https://github.com/nicx17/mimick.git)
cd mimick
flatpak-builder --user --install --force-clean build-dir io.github.nicx17.mimick.local.yml
flatpak run io.github.nicx17.mimick

```

---

## Documentation
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [User Guide](docs/USER_GUIDE.md)
- [Security Policy](SECURITY.md)

## Trust and Verification

Mimick currently publishes a few concrete trust signals:

- signed Flatpak repository metadata
- GitHub release assets with checksums
- CodeQL analysis in GitHub Actions
- CI checks for formatting, linting, tests, and dependency audits

If you install via Flatpak, verify the published signing fingerprint before trusting the repo:

`04E2 9556 E951 B2EA 15D3 A8EE 632E 1BC5 D956 579C`

## Contributing

Pull requests are welcome. See `CONTRIBUTING.md` for commit and style guidelines.

## Acknowledgments

* Application icon illustration by [Round Icons](https://unsplash.com/@roundicons) on Unsplash.

## License

GNU General Public License v3.0 — see [LICENSE](https://github.com/nicx17/mimick/blob/main/LICENSE).
