# Troubleshooting

## Installation Errors

### "Runtime Not Found" (`org.gnome.Platform`)

This occurs when Flatpak cannot find the GNOME Platform runtime required by Mimick. It typically happens on fresh installations of Ubuntu 25+, Fedora, or systems where Flathub is not enabled.

**Fix**: This means the Flathub remote is missing from your system, which Flatpak needs in order to download the runtime dependency. Please follow the instructions at [flathub.org/setup](https://flathub.org/setup) to add the Flathub remote to your system, and then retry the installation.

## API Key / Keyring Errors

### "Could Not Save API Key"

Mimick stores the API key using the oo7 keyring library. Inside Flatpak, this uses an encrypted file backend. Outside Flatpak, it uses the D-Bus Secret Service (GNOME Keyring or KWallet).

If you see this error:

1. Make sure your desktop keyring daemon is running (GNOME Keyring, KWallet, etc.)
2. If running inside Flatpak, check that the keyring file is not corrupted (see below)

### "IncorrectSecret" or "PartiallyCorruptedKeyring"

This can happen when upgrading from an older version of Mimick, or if the Flatpak portal secret has rotated (e.g., after reinstalling or rebuilding the Flatpak).

**Fix**: Delete the stale keyring file and relaunch. Mimick will create a fresh one:

```bash
rm -f ~/.var/app/io.github.nicx17.mimick/data/keyrings/default.keyring
flatpak run io.github.nicx17.mimick
```

Then re-enter and save your API key from the settings window.

### Keyring Access Issues (Headless Servers)
If you are running on a server without a desktop session (e.g., via SSH only), the native Keyring might fail to unlock the login keyring.
- **Solution:** Use `dbus-run-session` or configure `pam_gnome_keyring` to unlock on login.

## Display & UI Issues

### Tray Icon Missing
Some desktops restrict or hide legacy tray icons.
- **Wayland (GNOME/KDE) & Ubuntu 24+:** Modern desktop environments deprecate or heavily restrict legacy system trays. The app uses `ksni` (StatusNotifierItem via D-Bus).
- GNOME often needs the **AppIndicator/KStatusNotifierItem Support** extension installed.
- If tray support is unavailable, launching Mimick again should still intelligently detect the running instance and open the settings window instead.

### Notifications Look Wrong / No Progress Bars
If you see multiple individual notifications instead of a single updating bar:
- Some lightweight notification daemons do not support the `x-canonical-private-synchronous` hint, replacement, or progress hints well.
- **Solution:** Install a full-featured notification daemon like `dunst` (configured appropriately) or use a desktop environment like GNOME or KDE Plasma.

## Syncing & Upload Issues

### Files Are Not Syncing
If a file seems to be ignored completely, check:
1. the watch folder was selected through the app
2. the file extension is supported (Only Immich-compatible image and video formats are recognized)
3. the file finished writing to disk (temporary files are ignored)
4. the API key and server URLs are valid
5. folder rules are not excluding the file (hidden files, or max-size restrictions)

> **Check the Queue Inspector**
> The built-in Queue Inspector can tell you instantly if files are failing to upload.

> **Test Connection**
> If you suspect network issues, use the Ping Test dialog to test server reachability.

### Checksums / Deduplication Failures
If Immich re-uploads existing files:
- Ensure the server has finished processing existing assets.
- The app checks for `.device_asset_id` uniqueness from the server using a full 40-character SHA1 hex string. Verify that `sha1` checksums match.

### Mimick Stays Paused
If uploads do not resume on their own:
- Open the settings window and check the current status text. Mimick records the pause reason.
- If you manually paused it, use **Pause / Resume** from the tray or settings window.
- If **Pause on Metered Network** is enabled, Mimick may pause while `nmcli` reports a metered or guessed-metered connection.
- If **Pause on Battery Power** is enabled, Mimick may pause while the system appears to be running on battery according to `/sys/class/power_supply`.

## Logs & Diagnostics

### Clearing the Upload Queue (Local Cache)
If the application gets permanently stuck constantly trying to upload a corrupt or broken file on every start causing a queue blockage, you can manually delete the retry cache offline:
```bash
rm -f ~/.cache/mimick/retries.json
```
*(In Flatpak, use `~/.var/app/io.github.nicx17.mimick/cache/mimick/retries.json`)*

### Export a Diagnostics Bundle
Use `Export Diagnostics` from the Status page to collect:
- `summary.txt`
- `privacy-note.txt`
- `config.redacted.json`
- `status.redacted.json`
- `retries.redacted.json`
- `synced_index.redacted.json`

API keys, raw logs, full local paths, and raw server URLs are intentionally omitted. The bundle is written to a timestamped `mimick-diagnostics-*` folder.

### Useful Logs

Flatpak log:
```bash
tail -f ~/.var/app/io.github.nicx17.mimick/cache/mimick/mimick.log
```

Native log:
```bash
tail -f ~/.cache/mimick/mimick.log
```

If running as a systemd service (Native):
```bash
journalctl --user -u mimick -f
```

Terminal run:
```bash
cargo run
```

Both terminal and file logs include timestamps, levels, and source modules.

### Check Configuration Validity
Verify your config file is valid JSON (using native path example):
```bash
cat ~/.config/mimick/config.json | jq .
```
If `jq` reports an error, the file is malformed.

### Cache Files (Advanced)

Important runtime files (Flatpak paths shown, replace with `~/.cache/mimick/` for native):
- `~/.var/app/io.github.nicx17.mimick/cache/mimick/mimick.log`
- `~/.var/app/io.github.nicx17.mimick/cache/mimick/retries.json`
- `~/.var/app/io.github.nicx17.mimick/cache/mimick/synced_index.json`
- `~/.var/app/io.github.nicx17.mimick/cache/mimick/status.json`
