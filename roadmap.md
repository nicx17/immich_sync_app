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

### Priority Now

- [x] **Queue Inspector** — Failed items and recent queue activity are visible from the Controls page, with retry tools for recovery.
- [x] **Retry Controls** — `Retry all`, single-item `Retry`, and `Clear failed queue` are available from the queue inspector.
- [x] **Pause / Resume Sync** — Global pause works from the tray and settings window, with visible paused status and reason.
- [x] **Sync Now** — Manual rescan is available from the Controls page and tray menu.
- [x] **Per-Folder Rules** — Hidden-path filtering, extension allowlists, temp-file ignores, and optional max file size limits are supported.
- [x] **Diagnostics Bundle** — Logs, config/state snapshots, and recent queue details can be exported into one support-friendly bundle.
- [x] **Network / Power Awareness** — Uploads can defer on metered connections or while running on battery power.

### Next Wave

- [ ] **Upload Limits** — User-configurable concurrency limit, bandwidth cap, and quiet hours.
- [ ] **Notifications That Matter** — Summaries for sync success/failure, connectivity loss, permission loss, and album recreation events.
- [ ] **Health Dashboard** — Show last successful sync, current server route, watched folder count, pending queue size, retry count, and latest error.
- [ ] **Permission Health Checks** — Detect broken Flatpak portal access or lost folder permissions and guide the user to reauthorize them.
- [ ] **Safer Startup Catch-Up Controls** — Let users choose full catch-up, recent-only catch-up, or new-files-only behavior.
- [ ] **Per-Folder Status** — Track last sync time, pending count, target album, and last error per watch path.

### UX Improvements

- [ ] **First-Run Wizard** — Guided setup for server test, API key, first watch folder, and startup behavior.
- [ ] **Better Album Picker** — Searchable album list, recent albums, inline create-new, and a clearer `use folder name` option.
- [x] **Split Setup / Controls Window** — Separate configuration from live actions and keep footer actions visible while scrolling.
- [ ] **Status in Tray** — Surface idle / syncing / paused / offline / error state directly in the tray menu and tooltip.
- [ ] **Actionable Errors** — Translate generic failures into concrete guidance like invalid API key, missing album, network timeout, or folder access loss.
- [ ] **Dry Run / Preview Mode** — Show what would be uploaded before enabling a new folder or changing rules.
- [ ] **Verify Existing Remote State** — Audit a folder against the local sync index and highlight drift or mismatches.

### Platform & Packaging

- [ ] **Flatpak manifest hardening** — Keep improving portal-first packaging and release reliability for the hosted Flatpak repo.
- [ ] **Arch AUR submission** — Publish PKGBUILD to AUR as `mimick`.
- [ ] **ARM64 AppImage** — Cross-compile and package for Raspberry Pi / ARM desktops.

### Longer-Term Product Ideas

- [ ] **Complete folder sync** — Optional two-way sync mode that can reflect remote additions or deletions locally.
- [ ] **Exponential backoff** — Smarter retry scheduling instead of immediate replay on restart.
- [ ] **Progress notifications** — Desktop notifications with counts and outcomes, not just logs.
- [ ] **Tray icon dynamic states** — Distinct icons for idle, syncing, paused, and error conditions.
