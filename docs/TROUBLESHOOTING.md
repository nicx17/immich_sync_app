# Troubleshooting Guide

This guide covers common issues encountered while using `immich-sync`.

## Common Issues

### 1. System Tray Icon Not Appearing or App Crashes on Start
If the icon is missing or fails to initialize:
- **Wayland (GNOME/KDE) & Ubuntu 24+:** Modern desktop environments deprecate or heavily restrict legacy system trays. The app will automatically catch `pystray` initialization failures and gracefully fall back to **headless daemon mode**. 
- **Auto-Fallback Behavior:** If the tray fails, the daemon continues running in the background normally. If you launch the app directly from your desktop menu while the tray is disabled, it will intelligently open the Settings Window instead so you can still manage the application.
- **Environment Variables:** If running manually or via Systemd, ensure `XDG_CURRENT_DESKTOP` and `DISPLAY` are set correctly.

**Manual Workaround (Headless Mode):**
If you wish to force the app to skip trying to load the tray altogether:
```bash
python src/main.py --no-tray
```

### 2. Notifications Not showing Progress Bars
If you see multiple individual notifications instead of a single updating bar:
- Your notification server might not support the `x-canonical-private-synchronous` hint or `int:value` progress hints.
- **Solution:** Install a full-featured notification daemon like `dunst` (configured appropriately) or use a desktop environment like GNOME or KDE Plasma.

### 3. Checksums / Deduplication Failures
If Immich re-uploads existing files:
- Ensure the server has finished processing existing assets.
- Verify that `shad1` checksums match.
- The app checks for `DUPLICATE` response from the server; if the server APIs have changed, this check might fail.

### 4. Keyring Access Issues (Headless Servers)
If you are running on a server without a desktop session (e.g., via SSH only), `keyring` might fail to unlock the login keyring.
- **Solution:** Use `dbus-run-session` or configure `pam_gnome_keyring` to unlock on login. Alternatively, use a plaintext backend for `keyring` (less secure).

## Logs & Diagnostics

### Clearing the Upload Queue (Local Cache)
If the application gets permanently stuck constantly trying to upload a corrupt or broken file on every start causing a queue blockage, you can manually delete the retry cache offline:
```bash
rm -f ~/.cache/immich-sync/retries.json
```

### Viewing Logs (Systemd)
If running as a service:
```bash
journalctl --user -u immich-sync -f
```

### Manual Debugging
Run the application directly in a terminal to see `stdout` logs:
```bash
python src/main.py
```
Look for lines starting with `ERROR` or `CRITICAL`.

### Check Configuration Validity
Verify your config file is valid JSON:
```bash
cat ~/.config/immich-sync/config.json | jq .
```
If `jq` reports an error, the file is malformed.
