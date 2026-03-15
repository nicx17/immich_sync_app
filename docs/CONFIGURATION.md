# Configuration Guide

This document covers all configurable aspects of `mimick`.

## User Interface Configuration

The most convenient way to configure the application is via the built-in Settings Window.

1. Right-click the **System Tray Icon**.
2. Select **Settings**.
3. Modify your Internal/External URLs and API key.
4. Add or remove watch directories with the built-in folder picker.
5. Toggle **Run on Startup** if you want Mimick to launch automatically after login.
6. Click **Save & Restart**.
7. Use **Close** to hide the window or **Quit** to exit the app entirely.

`Save & Restart` now relaunches Mimick after writing the updated configuration so new folder watches and connectivity settings take effect immediately.

The settings window close button behaves like **Close** and keeps the background daemon running.

When Mimick starts, it also rescans the configured watch folders for media that has not been synced yet. A local sync index is used so unchanged files that were already synced are skipped quickly instead of being reuploaded every launch.

If you change the target album for a watched folder, Mimick can reassociate unchanged files to the new album on a later startup. If the previously used album was deleted, Mimick refreshes the album lookup and retries with the current configured album name.

## Manual Configuration (JSON)

The configuration is stored in a JSON file located at:

`~/.config/mimick/config.json`

### File Structure

```json
{
    "watch_paths": [
        "/home/user/Pictures",
        "/home/user/DCIM" 
    ],
    "internal_url": "http://192.168.1.10:2283",
    "external_url": "https://immich.example.com",
    "internal_url_enabled": true,
    "external_url_enabled": true,
    "run_on_startup": false
}
```

### Properties

| Key | Description | Example |
| :--- | :--- | :--- |
| `watch_paths` | A list of selected directories to monitor recursively. In Flatpak builds, these should be added from the settings window so portal access is granted; they may be stored as portal-backed paths under `/run/user/.../doc/...`. | `["/home/user/Screenshots"]` |
| `internal_url` | The LAN IP/Hostname of your Immich instance. Used when local connectivity is detected. | `http://192.168.1.10:2283` |
| `external_url` | The WAN/Public URL (reverse proxy). Used when away from home. | `https://photos.mydomain.com` |
| `internal_url_enabled` | Toggle allowing the Daemon to attempt LAN connectivity. | `true` |
| `external_url_enabled` | Toggle allowing the Daemon to attempt WAN connectivity. | `true` |
| `run_on_startup` | Whether Mimick should register itself for automatic login startup. | `false` |

## Local State and Cache Files

In addition to `config.json`, Mimick stores runtime state in the user cache directory:

- `~/.cache/mimick/mimick.log`: rotating application logs with timestamps, levels, and source modules.
- `~/.cache/mimick/retries.json`: queued retry items that could not be uploaded before shutdown.
- `~/.cache/mimick/synced_index.json`: the local sync index used by startup rescans to skip already-synced unchanged files and detect album-target changes.

## API Key Security

### Required API Key Permissions

When generating an API Key in the Immich Web UI (Account Settings > API Keys), you can restrict its permissions for better security. `mimick` requires the following minimum permissions:

- **Asset**: `Read` (to check for duplicates), `Create` (to upload new media)
- **Album**: `Read` (to list existing albums), `Create` (to create new albums), `Update` (to add uploaded media to albums)

### Keyring Storage

To prevent storing API keys in plain text, `mimick` uses the desktop's native keyring service (Libsecret on GNOME, KWallet on KDE).

- **Service Name**: `mimick`
- **Username**: `api_key`

If you need to manually intervene with the keyring (e.g., if you are running headless), you can use Python's `keyring` CLI or `seahorse` (GNOME Passwords and Keys).

**Using Python:**

```bash
python -c "import keyring; keyring.set_password('mimick', 'api_key', 'YOUR_API_KEY_HERE')"
```

## Systemd Service Configuration

The application runs as a user service. The service file is located at `~/.config/systemd/user/mimick.service`.

**Environment Variables:**
Ideally, configure environment variables in `~/.config/environment.d/mimick.conf`.

- `DISPLAY`: Usually `:0`
- `XDG_RUNTIME_DIR`: Required for DBus session bus access.

## Notification Configuration

The application uses `libnotify` via `notify-send`. It attempts to use hints (`int:value:progress`) to display progress bars.

- Ensure a notification daemon is running (e.g., `dunst`, `mako`, or DE-integrated).
- Some minimalist notification servers do not support progress bars or replacement; in this case, you may see multiple separate notifications.
