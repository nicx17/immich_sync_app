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

### 5. Mimick Stays Paused
If uploads do not resume on their own:
- Open the settings window and check the current status text. Mimick now records the pause reason.
- If you manually paused it, use **Pause / Resume** from the tray or settings window.
- If **Pause on Metered Network** is enabled, Mimick may pause while `nmcli` reports a metered or guessed-metered connection.
- If **Pause on Battery Power** is enabled, Mimick may pause while the system appears to be running on battery according to `/sys/class/power_supply`.

### 6. Files Never Enter the Queue
If a file seems to be ignored completely:
- Check the per-folder rules for that watch path.
- Hidden files and files inside hidden directories can now be excluded intentionally.
- Large files can be skipped by the max-size rule.
- Extension allowlists only accept matching file extensions after normalization.
- Temporary files are ignored until the final media filename appears.
- **New:** Only Immich-compatible image and video formats are recognized. See `formats.md` for the full list of supported extensions. Unsupported formats will be skipped automatically.

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

### Export a Diagnostics Bundle
If you need to file a bug or inspect the app state in one place, use **Export Diagnostics** from the settings window. The export creates a redacted `mimick-diagnostics-*` folder containing:
- `summary.txt`
- `config.redacted.json`
- `status.redacted.json`
- `retries.redacted.json`
- `synced_index.redacted.json`
- `privacy-note.txt`

The bundle intentionally omits API keys, raw logs, full local paths, and raw server URLs.

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
