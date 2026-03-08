# Mimick User Guide

Welcome to Mimick for Linux! This guide provides detailed instructions on how to use the application to automatically back up your local photo and video directories to your Immich server.

---

## 1. Getting Started

### The System Tray Icon

Once the application is running, a blue "Immich" icon will appear in your system tray (usually at the top right on GNOME/KDE).

*If you are using GNOME and don't see system tray icons, ensure you have the "AppIndicator and KStatusNotifierItem Support" GNOME extension enabled. Stock GNOME does not support StatusNotifier tray icons out of the box.*

Clicking on the tray icon reveals a menu:

* **Settings**: Opens the configuration and status window.
* **Quit**: Safely shuts down the application and stops all background syncing.

---

## 2. Configuring the Application

### Accessing Settings

Right-click the tray icon and select **Settings**, or launch with `mimick --settings`.

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

---

## 3. How Syncing Works

### Automatic Detection

Once configured, the application runs silently in the background. When you add a new photo to a watched folder, `mimick` detects it via Linux filesystem events (`inotify`):

1. Waits for the file size to stabilise (file is fully written to disk).
2. Calculates a SHA-1 checksum for deduplication.
3. Streams the file to Immich using the standard asset API.
4. Adds the asset to the configured album.

### Sync Status

Open the **Settings** window to see what is currently happening:

* **Idle** — Nothing is uploading. Shows total processed count.
* **Uploading** — Shows the current filename and a progress bar for the active batch.

### Offline Reliability

If an upload fails, the file is saved to `~/.cache/mimick/retries.json`. On the next launch, any persisted retries are automatically re-queued and uploaded.

---

## 4. Frequently Asked Questions

**Q: Will this delete my local files?**
No. Mimick is strictly one-way (backup mode). It reads local files and uploads them. It never modifies or deletes files on your local machine.

**Q: Are sidecar files supported?**
Currently, Mimick ignores metadata sidecar files (`.xmp`, etc.). Immich has limited sidecar support via the standard API, so they are filtered to prevent clutter.

**Q: What happens if my server is offline?**
The upload will fail gracefully and the file is saved to the retry queue (`~/.cache/mimick/retries.json`). On next launch, it will be automatically retried.

**Q: The tray icon does not appear on GNOME.**
GNOME requires the "AppIndicator and KStatusNotifierItem Support" extension. Install it from the GNOME Extensions website. Without it, the warning `Watcher(ServiceUnknown)` is expected and harmless — the app still runs fully in the background.
