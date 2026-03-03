# Configuration Guide

This document covers all configurable aspects of `immich-sync`.

## User Interface Configuration
The most convenient way to configure the application is via the built-in Settings Window.
1.  Right-click the **System Tray Icon**.
2.  Select **Settings**.
3.  Modify your Internal/External URLs and API key.
4.  Add or remove watch directories.
5.  Click **Save & Restart**.

## Manual Configuration (JSON)
The configuration is stored in a JSON file located at:

`~/.config/immich-sync/config.json`

### File Structure
```json
{
    "watch_paths": [
        "/home/user/Pictures",
        "/home/user/DCIM" 
    ],
    "internal_url": "http://192.168.1.10:2283",
    "external_url": "https://immich.example.com"
}
```

### Parameter Details

| Key | Description | Example |
| :--- | :--- | :--- |
| `watch_paths` | A list of local directories to monitor recursively. | `["/home/user/Screenshots"]` |
| `internal_url` | The LAN IP/Hostname of your Immich instance. Used when local connectivity is detected. | `http://192.168.1.10:2283` |
| `external_url` | The WAN/Public URL (reverse proxy). Used when away from home. | `https://photos.mydomain.com` |

## API Key Security
To prevent storing API keys in plain text, `immich-sync` uses the desktop's native keyring service (Libsecret on GNOME, KWallet on KDE).

- **Service Name**: `immich-sync`
- **Username**: `api_key`

If you need to manually intervene with the keyring (e.g., if you are running headless), you can use Python's `keyring` CLI or `seahorse` (GNOME Passwords and Keys).

**Using Python:**
```bash
python -c "import keyring; keyring.set_password('immich-sync', 'api_key', 'YOUR_API_KEY_HERE')"
```

## Systemd Service Configuration
The application runs as a user service. The service file is located at `~/.config/systemd/user/immich-sync.service`.

**Environment Variables:**
Ideally, configure environment variables in `~/.config/environment.d/immich-sync.conf`.
- `DISPLAY`: Usually `:0`
- `XDG_RUNTIME_DIR`: Required for DBus session bus access.

## Notification Configuration
The application uses `libnotify` via `notify-send`. It attempts to use hints (`int:value:progress`) to display progress bars.
- Ensure a notification daemon is running (e.g., `dunst`, `mako`, or DE-integrated).
- Some minimalist notification servers do not support progress bars or replacement; in this case, you may see multiple separate notifications.
