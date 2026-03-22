# Mimick User Guide

Welcome to Mimick for Linux! This guide provides detailed instructions on how to use the application to automatically back up your local photo and video directories to your Immich server.

---

## 1. Getting Started

### The System Tray Icon

Once the application is running, a blue "Immich" icon will appear in your system tray (usually at the top right on GNOME/KDE).

*If you are using GNOME and don't see system tray icons, ensure you have the "AppIndicator and KStatusNotifierItem Support" GNOME extension enabled. Stock GNOME does not support StatusNotifier tray icons out of the box.*

Clicking on the tray icon reveals a menu:

* **Settings**: Opens the configuration and status window.
* **Pause / Resume**: Temporarily stop uploads without quitting the app, then continue later.
* **Sync Now**: Trigger an immediate rescan of watched folders and queue any eligible files right away.
* **Quit**: Safely shuts down the application and stops all background syncing.

You can also use the launcher action for **Quit Mimick** to stop the already-running app without opening the settings window.

---

## 2. Configuring the Application

### Accessing Settings

Right-click the tray icon and select **Settings**, or launch with `mimick --settings`.

The settings window is split into two pages:

* **Setup**: server details, behavior switches, watch folders, and folder rules
* **Controls**: sync status, queue tools, pause/resume, manual sync, and diagnostics export

### Connectivity & Server Details

1. **Internal URL (LAN)**: Enter the local IP address of your Immich server (e.g., `http://192.168.1.10:2283`). Can be toggled on/off.
2. **External URL (WAN)**: Enter the public address (e.g., `https://immich.yourdomain.com`). Can be toggled on/off. At least one URL must always remain enabled.
3. **API Key**:
    * Open your Immich Web Interface in a browser.
    * Go to **Account Settings** → **API Keys**.
    * Click **New API Key**, give it a name (like "Linux Desktop"), and click Create.
    * Copy the key and paste it into the API Key field in Mimick.
    * *The key is stored in your system's secure keyring (libsecret). It is never written to disk in plain text.*

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

### Startup Behavior

Use the **Run on Startup** switch in the **Behavior** section if you want Mimick to launch automatically when you log in.

* Flatpak builds ask the desktop for permission using the background portal.
* Native builds create `~/.config/autostart/io.github.nicx17.mimick.desktop`.

You can also enable:

* **Pause on Metered Network**: Mimick defers uploads when the active connection appears metered.
* **Pause on Battery Power**: Mimick defers uploads while the system appears to be running on battery.

### Saving Changes

Click **Save & Restart** after changing settings. Mimick saves the updated configuration, closes the current instance, and launches a fresh one so the new watcher and connection settings take effect immediately.

The footer keeps **Close**, **Quit**, and **Save & Restart** visible even if the current page needs scrolling.

### Closing vs Quitting

The settings window has separate actions for hiding the window and quitting the whole app:

* **Close** hides the settings window and keeps Mimick running in the background.
* **Quit** fully exits Mimick.
* The window titlebar close button behaves the same as **Close**.

### Controls Page

The **Controls** page groups the live actions you may want while Mimick is already running:

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

## 4. Frequently Asked Questions

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
