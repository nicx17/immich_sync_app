# Immich Auto-Sync User Guide

Welcome to Immich Auto-Sync for Linux! This guide provides detailed instructions on how to use the application to automatically back up your local photo and video directories to your Immich server.

---

## 1. Getting Started

### The System Tray Icon

Once the application is running, a blue "Immich" icon will appear in your system tray (usually at the top right on GNOME/KDE, or bottom right on other desktop environments).
*If you are using GNOME and don't see system tray icons, ensure you have the "AppIndicator and KStatusNotifierItem Support" extension enabled.*

Clicking on the tray icon reveals a menu with two primary options:

* **Settings**: Opens the configuration and status window.
* **Quit**: Safely shuts down the application and stops all background syncing.

---

## 2. Configuring the Application

### Accessing Settings

If you didn't configure the application on the first run, right-click the tray icon and select **Settings**.

### Connectivity & Server Details

1. **Internal URL (LAN)**: Enter the local IP address of your Immich server if it sits on the same network as your desktop (e.g., `http://192.168.1.10:2283`). This allows for high-speed local transfers when you are at home.
2. **External URL (WAN)**: Enter the public address of your server if you expose it to the internet (e.g., `https://immich.yourdomain.com`). The application will fall back to this address if you take your laptop on the road.
3. **API Key**:
    * Open your Immich Web Interface in a browser.
    * Go to **Account Settings** (profile icon in the top right) -> **API Keys**.
    * Click **New API Key**, give it a name (like "Linux Desktop"), and click Create.
    * Copy the long string of text and paste it into the application's API Key field.
    * *Note: This key is never stored in plain text. It is saved in your system's secure keyring.*

**Test Connection**: Click this button to verify that the app can successfully reach your server and to see which URL (Internal or External) it is currently using.

### Choosing Folders to Watch

1. Under **Watch Folders**, click **+ Add Folder**.
2. Select a local directory (e.g., `~/Pictures`, `~/Videos/Exports`).
3. The application will monitor these folders recursively (including all sub-folders).
4. *Smart Albums Feature*: If you drop files into `~/Pictures/Vacation`, the app will automatically create an album in Immich named "Vacation" and add the photos to it!

---

## 3. How Syncing Works

### Automatic Detection

Once configured, the application runs silently in the background. When you add a new photo to one of your watched folders, `immich-sync` detects it instantly via Linux filesystem events (`inotify`).

* It waits a moment to ensure the file has finished downloading or rendering.
* It calculates a checksum of the photo.
* It uploads it to your Immich server in the background.

### Sync Status

You can check to see what the application is currently doing by opening the **Settings** window.
The "Sync Status" section features a progress bar and label indicating:

* **Status: Idle**: Nothing is currently uploading. It will show the total number of files processed.
* **Uploading**: Will list the specific filename being transferred and show a dynamic progress bar tracking the batch.

### Notifications

As files are uploaded, you may see a native desktop notification appear on your screen showing the upload progress.

---

## 4. Frequently Asked Questions

**Q: Will this delete my local files?**
No. This is strictly a "One-Way Sync" (Backup mode). It reads your local files and uploads them. It will *never* delete a file on your local machine, and it will *never* download files deleted from the Immich web interface back to your computer.

**Q: Are sidecar files supported?**
Currently, `immich-sync` intentionally ignores metadata sidecar files (like `.xmp`). Immich has limited backend support for sidecars uploaded via the standard API without bulk CLI tools, so we filter them to prevent clutter.

**Q: What happens if my server is offline?**
The application will detect the connection failure. Right now, it relies on connection retries. Advanced offline queuing is currently on the development roadmap.
