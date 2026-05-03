# Configuration and First Run

Welcome to Mimick for Linux! This guide provides detailed instructions on how to configure and use the application to automatically back up your local photo and video directories to your Immich server.

---

## 1. Getting Started

### The System Tray Icon

Once the application is running, the Mimick tray icon will appear in your system tray (usually at the top right on GNOME/KDE).

*If you are using GNOME and don't see system tray icons, ensure you have the "AppIndicator and KStatusNotifierItem Support" GNOME extension enabled. Stock GNOME does not support StatusNotifier tray icons out of the box.*

Clicking on the tray icon reveals a menu:

* **Settings**: Opens the configuration and status window.
* **Pause / Resume**: Temporarily stop uploads without quitting the app, then continue later.
* **Sync Now**: Trigger an immediate rescan of watched folders and queue any eligible files right away.
* **Quit**: Safely shuts down the application and stops all background syncing.

You can also use the launcher action for **Quit Mimick** to stop the already-running app without opening the settings window.

The tray icon uses a single static icon regardless of sync state. To check current status, open the Settings window and look at the Status page. Dynamic tray icon states (idle / syncing / paused / error) are not yet implemented.

---

## 2. Configuring the Application

### Accessing Settings

Right-click the tray icon and select **Settings**, or launch with `mimick --settings`.

The settings window is split into two pages:

* **Settings**: server details, behavior switches, watch folders, and folder rules
* **Status**: sync status, queue tools, pause/resume, manual sync, and diagnostics export

The window follows your desktop appearance preference, so it can render in either light mode or dark mode depending on the system theme.

### Connectivity & Server Details

1. **Internal URL (LAN)**: Enter the local IP address of your Immich server (e.g., `http://192.168.1.10:2283`). Can be toggled on/off.
2. **External URL (WAN)**: Enter the public address (e.g., `https://immich.yourdomain.com`). Can be toggled on/off. At least one URL must always remain enabled.
3. **API Key**:
    * Open your Immich Web Interface in a browser.
    * Go to **Account Settings** → **API Keys**.
    * Click **New API Key**, give it a name (like "Linux Desktop"), and click Create.
    * Make sure the key includes **Asset Read/Create/Update/Download** and **Album Read/Create/Update** permissions.
    * Copy the key and paste it into the API Key field in Mimick.
    * *The key is stored securely using the `oo7` crate (encrypted portal file in Flatpak, or D-Bus Secret Service native).*

**Test Connection**: Verifies connectivity by pinging the Immich `/api/server/ping` endpoint, confirming a valid `{"res": "pong"}` JSON response to ensure you are talking to an actual Immich server rather than a captive portal.

### Choosing Folders to Watch

1. Under **Watch Folders**, click **+ Add Folder**.
2. Select a local directory (e.g., `~/Pictures`, `~/Videos/Exports`).
3. The application monitors these folders recursively.
4. **Album Selection**: Each folder row has a dropdown to assign an Immich album. Choose an existing album, type a custom name (a new album will be created), or leave as "Default (Folder Name)" to auto-name from the folder.
5. **Folder Rules**: Each folder can open a rules dialog for extra filtering:
    * **Ignore hidden files and folders**
    * **Maximum file size (MB)**
    * **Allowed extensions** as a comma-separated allowlist like `jpg, png, mp4`

Flatpak builds only have access to folders that you add through this picker. If you are upgrading from an older build that had wider filesystem access, remove and re-add existing watch folders once.

Portal-backed folders may appear by folder name in the UI and logs instead of showing the raw `/run/user/.../doc/...` sandbox path.

### Nested Watch Folders

You can add both a parent folder and a subfolder as separate watch entries. Mimick uses the most specific matching path when deciding which album and rules to apply to a file.

For example, if you watch `~/Pictures` (album: **All Photos**) and also `~/Pictures/iPhone` (album: **iPhone**), files inside `~/Pictures/iPhone` use the **iPhone** album and its rules. Files elsewhere under `~/Pictures` use the **All Photos** album.

There is no limit on nesting depth. If a file matches multiple watch paths, the longest matching prefix wins.

### Startup Behavior

Use the **Run on Startup** switch in the **Behavior** section if you want Mimick to launch automatically when you log in.

* Flatpak builds ask the desktop for permission using the background portal.
* Native builds create `~/.config/autostart/dev.nicx.mimick.desktop`.

You can also enable:

* **Pause on Metered Network**: Mimick defers uploads when the active connection appears metered.
* **Pause on Battery Power**: Mimick defers uploads while the system appears to be running on battery.

### Background Sync

The **Background Sync** switch controls whether Mimick continues running when the settings window is closed.

* **Enabled**: closing the window hides it. Mimick keeps syncing in the background and remains accessible from the tray.
* **Disabled (default)**: closing the window quits the application entirely if no upload is in progress. Mimick only runs while the window is open.

If you disable Background Sync and also enable Run on Startup, Mimick will launch on login but quit as soon as you close the window. Re-opening it from the launcher starts it again.

### Quiet Hours

The **Quiet Hours** switch in the Behavior section pauses uploads during a nightly window. When enabled, two spinners let you set a start hour and an end hour (0–23, using your local clock).

* Uploads in progress when the quiet window begins are paused at the next worker cycle, not interrupted mid-file.
* Uploads resume automatically when the local clock passes the end hour.
* Wrapping windows are supported: setting start to `22` and end to `6` pauses from 22:00 until 06:00 the next morning.
* Setting start and end to the same hour disables the window even if the switch is on.
* Quiet hours interact with other pause conditions — if the app is also manually paused or paused by a metered-network policy, it stays paused for the other reason after the quiet window ends.

The quiet hours check runs on the local system clock. Timezone changes while the app is running take effect on the next worker cycle.

### Startup Catch-Up Mode

The **Startup Catch-Up** dropdown in the Behavior section controls how thoroughly Mimick rescans watch folders when it launches.

| Mode | Behaviour |
| :--- | :--- |
| **Full** | Scans every file in every watch folder regardless of when it was last modified. Suitable if you want a complete audit on each launch. Slowest on large folders. |
| **Recent Only** | Scans files modified in approximately the last 7 days. Faster than Full but may miss older files added while the app was not running. |
| **New Files Only** | Only processes files not already present in the local sync index. Fastest; does not re-check files that were previously seen even if their content changed. |

The default is **Full**. If startup is slow on a large library you can switch to **New Files Only** once the initial sync is complete.

### Saving Changes

Click **Save Changes** after changing settings. Mimick saves the updated configuration and applies it to the running app immediately, including updated server URLs, watch folders, worker count, and pause policies.

The footer keeps **Close**, **Quit**, and **Save Changes** visible even if the current page needs scrolling.

### Closing vs Quitting

The settings window has separate actions for hiding the window and quitting the whole app:

* **Close** hides the settings window and keeps Mimick running in the background.
* **Quit** fully exits Mimick.
* The window titlebar close button behaves the same as **Close**.

### Status Page

The **Status** page groups the live actions you may want while Mimick is already running:

* **Sync Now** to trigger an immediate watched-folder scan
* **Pause / Resume** to stop and continue uploads manually
* **Queue Inspector** for failure recovery
* **Export Diagnostics** for support bundles

### Queue Inspector and Recovery

Inside it you can:

* review recent queue events from the current session
* see failed items waiting to be retried
* retry one failed item
* retry all failed items
* clear the failed queue

This is useful when a server outage, permission issue, or bad file temporarily blocks uploads.

### Diagnostics Export

Use **Export Diagnostics** in the settings window when you need a support snapshot.

The export creates a timestamped `mimick-diagnostics-*` folder containing redacted support files:

* `summary.txt`
* `config.redacted.json`
* `status.redacted.json`
* `retries.redacted.json`
* `synced_index.redacted.json`
* `privacy-note.txt`

API keys, raw logs, full local paths, and raw server URLs are intentionally omitted.

---

## Library View (Optional)

Mimick includes an opt-in library browser for albums, Explore, and search.

Enable it in **Settings → Behavior → Enable Library View**, then restart. See the [Library View user guide](Library-View-User-Guide) for full usage documentation.

**Extra permissions:** Library browsing requires **Asset Read** and downloads require **Asset Download**.

---

## 3. How Syncing Works

### Automatic Detection

Once configured, the application runs silently in the background. It handles syncing in two ways:

1. On startup, Mimick rescans watched folders for media that has not been synced yet.
2. While running, Mimick watches those folders for newly added or changed media.

For live changes, `mimick` detects files via filesystem monitoring:

1. Waits for the file size to stabilise (file is fully written to disk).
2. Calculates a SHA-1 checksum for deduplication.
3. Streams the file to Immich using the standard asset API.
4. Adds the asset to the configured album.

### Existing Files and Reassignment

Mimick keeps a local sync index so it can avoid reprocessing files that are already known to be synced.

* Unchanged files that were already synced are skipped during startup rescans.
* Files whose content changed are rehashed and uploaded again.
* If you change the target album for a watched folder, Mimick can reassociate unchanged files to the new album on a later startup without needing to reupload the media data.
* If the previously targeted album was deleted, Mimick refreshes the album mapping and retries using the current configured album name.

### Sync Status

Open the **Settings** window to see what is currently happening:

* **Idle** — Nothing is uploading. Shows total processed count.
* **Uploading** — Shows the current filename and a progress bar for the active batch.
* **Paused** — Mimick is intentionally holding uploads. The UI shows the pause reason, such as a manual pause, metered network, or battery-power policy.

### Offline Reliability

If an upload fails, the file is saved to `~/.cache/mimick/retries.json`. On the next launch, any persisted retries are automatically re-queued and uploaded.

Files blocked by folder rules are skipped before they ever enter the queue. Temporary files are also ignored until the final media file exists.

---

## 4. Advanced Configuration 

### Manual Configuration (JSON)

The configuration is stored in a JSON file located at:

`~/.config/mimick/config.json`

(Inside flatpak, this path is `~/.var/app/dev.nicx.mimick/config/mimick/config.json`).

### File Structure

```json
{
    "watch_paths": [
        "/home/user/Pictures",
        {
            "path": "/home/user/DCIM",
            "album_name": "Phone Uploads",
            "rules": {
                "ignore_hidden": true,
                "max_file_size_mb": 500,
                "allowed_extensions": ["jpg", "png", "mp4"]
            }
        }
    ],
    "internal_url": "http://192.168.1.10:2283",
    "external_url": "https://immich.example.com",
    "internal_url_enabled": true,
    "external_url_enabled": true,
    "run_on_startup": false,
    "pause_on_metered_network": false,
    "pause_on_battery_power": false
}
```

### Properties

| Key | Description | Example |
| :--- | :--- | :--- |
| `watch_paths` | A list of selected directories to monitor recursively. Entries may be plain strings for older configs or objects containing `path`, optional album targeting fields, and `rules`. In Flatpak builds, add them from the settings window so portal access is granted; they may be stored as portal-backed paths under `/run/user/.../doc/...`. | `["/home/user/Screenshots"]` |
| `internal_url` | The LAN IP/Hostname of your Immich instance. Used when local connectivity is detected. | `http://192.168.1.10:2283` |
| `external_url` | The WAN/Public URL (reverse proxy). Used when away from home. | `https://photos.mydomain.com` |
| `internal_url_enabled` | Toggle allowing the Daemon to attempt LAN connectivity. | `true` |
| `external_url_enabled` | Toggle allowing the Daemon to attempt WAN connectivity. | `true` |
| `run_on_startup` | Whether Mimick should register itself for automatic login startup. | `false` |
| `pause_on_metered_network` | Whether uploads should pause while the active network connection appears metered. | `false` |
| `pause_on_battery_power` | Whether uploads should pause while the system appears to be running on battery. | `false` |
| `library_view_enabled` | Whether the in-app library view opens instead of the settings window. | `false` |
| `download_target_path` | Target folder for library downloads (chosen on first download). | `"/home/user/Pictures/Downloads"` |
| `library_preview_full_resolution` | Load full-resolution originals in the lightbox instead of previews. | `false` |
| `library_thumbnail_cache_mb` | RAM cap for decoded thumbnails (0 = default 80 MB). | `80` |

### `watch_paths` Object Form

When a watch path is stored as an object, these fields can appear:

| Key | Description |
| :--- | :--- |
| `path` | Absolute or portal-backed directory path being watched. |
| `album_id` | Optional cached Immich album ID. |
| `album_name` | Optional album name or user-entered target label. |
| `rules.ignore_hidden` | Skip any file inside a hidden path component such as `.cache` or `.stfolder`. |
| `rules.max_file_size_mb` | Optional maximum file size in megabytes. Files larger than this are skipped before queueing. |
| `rules.allowed_extensions` | Optional allowlist of extensions. Values are normalized case-insensitively and leading dots are ignored. |

### API Key Security & Required Permissions

When generating an API Key in the Immich Web UI (Account Settings > API Keys), you can restrict its permissions for better security. `mimick` requires the following minimum permissions:

- **Asset**: `Read` (to check for duplicates), `Create` (to upload new media), `Update` (to reapply final asset timezone metadata after upload), `Download` (for library view downloads)
- **Album**: `Read` (to list existing albums), `Create` (to create new albums), `Update` (to add uploaded media to albums)

### Systemd Service Configuration (Native Only)

If running native, the application can run as a user service. The service file is located at `~/.config/systemd/user/mimick.service`.

**Environment Variables:**
Ideally, configure environment variables in `~/.config/environment.d/mimick.conf`.

- `DISPLAY`: Usually `:0`
- `XDG_RUNTIME_DIR`: Required for DBus session bus access.

---

## 5. Frequently Asked Questions

**Q: Will this delete my local files?**
No. Mimick is strictly one-way (backup mode). It reads local files and uploads them. It never modifies or deletes files on your local machine.

**Q: Are sidecar files supported?**
Currently, Mimick ignores metadata sidecar files (`.xmp`, etc.). Immich has limited sidecar support via the standard API, so they are filtered to prevent clutter.

**Q: What happens if my server is offline?**
The upload will fail gracefully and the file is saved to the retry queue (`~/.cache/mimick/retries.json`). On next launch, it will be automatically retried.

**Q: Why is Mimick paused even though I did not click Pause?**
Check the **Behavior** section. If **Pause on Metered Network** or **Pause on Battery Power** is enabled, Mimick can pause itself automatically when those conditions are detected.

**Q: What does Sync Now do?**
It reruns the watched-folder scan immediately so you do not need to restart Mimick to pick up missed or newly eligible files.

**Q: The tray icon does not appear on GNOME.**
GNOME requires the "AppIndicator and KStatusNotifierItem Support" extension. Install it from the GNOME Extensions website. Without it, the warning `Watcher(ServiceUnknown)` is expected and harmless — the app still runs fully in the background.

**Q: Mimick quits when I close the window.**
Background Sync is disabled. Enable it in **Settings → Behavior → Background Sync** and save. After that, closing the window hides it instead of exiting.

**Q: Uploads are paused but I did not set quiet hours or enable any pause policy.**
Open the Status page and check the status text — it shows the pause reason. Common causes: the quiet-hours window is active, a metered-network or battery-power policy triggered, or the app was paused manually from the tray.

**Q: The startup scan is slow.**
Switch **Startup Catch-Up Mode** to **New Files Only** in Settings → Behavior once your initial sync is complete. This skips files already in the sync index and only processes genuinely new additions.

**Q: I have two watch folders pointing at the same parent and subfolder. Which album does a file use?**
The most specific (longest) matching path wins. A file at `~/Pictures/iPhone/img.jpg` uses the album configured for `~/Pictures/iPhone`, not the one for `~/Pictures`.
