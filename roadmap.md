# Mimick for Linux: Feature Roadmap

## Completed

### Core Sync Engine
- [x] Monitor directories via Linux `inotify` (`notify` crate).
- [x] File write-completion detection (size stabilisation over 3 consecutive polls).
- [x] SHA-1 checksumming per file for Immich deduplication (64KB chunked, low RAM).
- [x] One-way sync — never delete local files or download from server.
- [x] File type whitelist (JPG, PNG, HEIC, MP4, MOV, GIF, WEBP, TIFF, RAW, ARW, DNG). Sidecars ignored.
- [x] 10 concurrent streaming upload workers (constant RAM use regardless of file size).
- [x] Persistent retry queue (`~/.cache/mimick/retries.json`) — failed uploads survive reboots.

### Immich API Client
- [x] Smart URL routing — LAN first, WAN fallback.
- [x] Pre-upload deduplication via SHA-1 + 409 Conflict detection.
- [x] Multipart streaming upload (disk → network, no full RAM load).
- [x] ISO 8601 UTC timestamps (no chrono dependency, pure arithmetic).
- [x] Album auto-creation from local folder name.
- [x] Custom album selection per watch folder (existing or new).
- [x] HTTP error handling (413, 502, 504).

### Configuration & Security
- [x] Config file at `~/.config/mimick/config.json` (serde_json).
- [x] API key stored via `secret-tool` (libsecret) — never written to disk in plain text.
- [x] Multiple watch directories with per-folder album config.
- [x] `WatchPathEntry` supports both plain path strings and per-folder album configs.

### Settings UI
- [x] GTK4 + Libadwaita native UI (dark mode, `adw::ApplicationWindow`).
- [x] Internal/External URL fields with toggles (at least one must stay enabled — validated).
- [x] Test Connection button (async ping, no UI freeze).
- [x] Watch folders list with per-row album `DropDown` + custom name `Entry`.
- [x] Live sync status row and progress bar (polling `status.json`).
- [x] Save & Restart flow.

### System Tray
- [x] StatusNotifierItem tray via `ksni` crate.
- [x] Graceful fallback when `org.kde.StatusNotifierWatcher` is unavailable (GNOME without extension).

### Desktop Integration
- [x] `systemd` user service (`setup/mimick.service`) with journal logging.
- [x] `.desktop` file with Settings action (`setup/mimick.desktop`).
- [x] Native desktop notifications (`libnotify`).
- [x] PKGBUILD for Arch Linux / AUR.
- [x] AppImage packaging (`build_test_appimage.sh`).

### Rust Port (v2.0)
- [x] Full rewrite from Python to Rust (Tokio + GTK4-rs + Libadwaita-rs).
- [x] No Python runtime dependency — single statically-linked binary.
- [x] 11 unit tests across `api_client`, `config`, `monitor`, `queue_manager`, `state_manager`.
- [x] All GTK4 widgets updated to 4.10+ standards (no deprecated `ComboBoxText`, `MessageDialog`).

---

## Planned

### Next Up

- [ ] **Fix bug** multiple settings window spawning. lock to single window
- [ ] **Headless Operation** should operate without window being visible.. background process

- [ ] **Flatpak manifest** — Flathub-ready packaging with `xdg-desktop-portal` filesystem constraints.
- [ ] **Complete folder sync** — Two-way sync mode: toggle to sync remote deletions/additions back to local.
- [ ] **Arch AUR submission** — Publish PKGBUILD to AUR as `mimick`.

### Future
- [ ] **Exponential backoff** on retries (currently immediate re-queue on next launch).
- [ ] **Progress notification** — Native desktop notification with upload count, not just log.
- [ ] **Tray icon dynamic states** — Distinct icons for idle / uploading / error.
- [ ] **Selective sync** — File type filter toggles per watch folder in the UI.
- [ ] **ARM64 AppImage** — Cross-compile and package for Raspberry Pi / ARM desktops.
