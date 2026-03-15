# Troubleshooting Guide

This guide covers common issues encountered while using `mimick`.

## Common Issues

### 1. System Tray Icon Not Appearing or App Crashes on Start
If the icon is missing or fails to initialize:
- **Wayland (GNOME/KDE) & Ubuntu 24+:** Modern desktop environments deprecate or heavily restrict legacy system trays. The app uses `ksni` (StatusNotifierItem via D-Bus). 
- **Auto-Fallback Behavior:** If the tray fails or your desktop doesn't support AppIndicators, the daemon continues running in the background normally. If you launch the app directly from your desktop menu while the tray is disabled, it will intelligently detect the running instance and open the Settings Window instead so you can still manage the application.

### 2. Notifications Not showing Progress Bars
If you see multiple individual notifications instead of a single updating bar:
- Your notification server might not support the `x-canonical-private-synchronous` hint or `int:value` progress hints.
- **Solution:** Install a full-featured notification daemon like `dunst` (configured appropriately) or use a desktop environment like GNOME or KDE Plasma.

### 3. Checksums / Deduplication Failures
If Immich re-uploads existing files:
- Ensure the server has finished processing existing assets.
- Verify that `sha1` checksums match.
- The app checks for `.device_asset_id` uniqueness from the server using a full 40-character SHA1 hex string.

### 4. Keyring Access Issues (Headless Servers)
If you are running on a server without a desktop session (e.g., via SSH only), `secret-tool` might fail to unlock the login keyring.
- **Solution:** Use `dbus-run-session` or configure `pam_gnome_keyring` to unlock on login.

## Logs & Diagnostics

### Clearing the Upload Queue (Local Cache)
If the application gets permanently stuck constantly trying to upload a corrupt or broken file on every start causing a queue blockage, you can manually delete the retry cache offline:
```bash
rm -f ~/.cache/mimick/retries.json
```

### Viewing Logs (Systemd)
If running as a service:
```bash
journalctl --user -u mimick -f
```

### View Persistent File Logs
The application writes rotating debug logs to `.cache`. If something breaks without notification:
```bash
tail -f ~/.cache/mimick/mimick.log
```
Each log line includes a timestamp, level, and source module.

### Manual Debugging
Run the application directly in a terminal to see `stdout` logs:
```bash
mimick
# or if developing:
cargo run
```
Terminal logs also include timestamps. Look for lines starting with `ERROR` or `WARN`.

### Check Configuration Validity
Verify your config file is valid JSON:
```bash
cat ~/.config/mimick/config.json | jq .
```
If `jq` reports an error, the file is malformed.
