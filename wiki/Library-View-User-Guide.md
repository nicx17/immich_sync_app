# Library View

The library view is an optional in-app browser for your Immich server's assets, albums, and Explore categories. It replaces the default settings window as the main window when enabled.

---

## Enabling the Library View

1. Open **Settings → Behavior → Enable Library View**.
2. Save and restart Mimick.

The library window opens instead of the settings window on the next launch. Settings remain accessible from the header bar gear button.

---

## Layout

The window uses a sidebar + content split.

**Sidebar** (toggle with **F9** or the sidebar button in the header):

- **Photos** — opens the timeline grid
- **Explore** — People, Places, Things
- **Albums** — album landing page
- album entries listed below for quick navigation

**Header bar controls (right side):**

- source selector dropdown (**Remote / Local / Unified**)
- sort selector (**Newest / Filename / File Type / Sync State**)
- search entry with search mode selector
- Timeline toggle button
- select-mode toggle button
- refresh button
- gear button (opens Settings)

---

## Browsing Assets

### Grid and Pagination

Assets load in pages of 50. Scroll to the bottom of the grid to load the next page. The footer label shows the current count.

Thumbnails are loaded asynchronously. A placeholder is shown while the thumbnail downloads. Decoded thumbnails are cached in RAM up to the configured limit (`library_thumbnail_cache_mb`, default 80 MB).

### Sorting

Use the sort dropdown to order the current view by:

- **Newest** — most recently created first
- **Filename** — alphabetical
- **File Type** — grouped by MIME type
- **Sync State** — local-only assets first, then synced

Sort applies to the current source and page.

### Timeline View

The **Timeline** button in the header switches the Remote source between a standard paged grid and a timeline layout that groups assets by date. Timeline is only available for the Remote source; it is hidden when Local or Unified is active.

---

## Sources

The source dropdown controls which assets the grid shows.

| Source | What it shows |
| :--- | :--- |
| **Remote** | Assets fetched from the Immich server |
| **Local** | Files in your configured watch folders, enumerated directly |
| **Unified** | Remote assets merged with local sync state |

Switching sources clears any active search and reloads from page 1.

**Local source notes:**

- Local enumeration walks your watch folders using the same extension filter as the sync engine.
- No checksum is computed during enumeration, so Local mode does not indicate whether a file has been uploaded — use Unified for that.
- Files matched via the sync index show their album assignment and sync state.

---

## Searching

The search entry appears in the header bar. Enter a query and press Enter or wait for the field to commit.

### Search Modes (Remote only)

Use the mode dropdown next to the search entry:

| Mode | What it searches | Server requirement |
| :--- | :--- | :--- |
| **Filename** | Filename and EXIF metadata fields | None |
| **Smart Search** | CLIP-based semantic/natural-language similarity | Immich ML service running |
| **OCR** | Text extracted from images | Immich ML service running |

- Smart Search and OCR both require the Immich machine-learning service to be enabled and healthy on your server. Queries against a server without ML will return empty results or an error.
- Filename mode works without ML and is the fastest option.
- Clearing the search entry returns to the previous non-search source.

**Local and Unified search** always uses filename matching regardless of the mode selector. The mode selector is hidden when Local or Unified is active.

---

## Explore

Select **Explore** in the sidebar to open the Explore view.

Three sections are populated from the Immich server:

- **People** — a horizontal row of recognised person tiles with their name and a representative thumbnail. Requires face recognition to be enabled on the server.
- **Places** — city/location tiles for assets with geolocation EXIF data.
- **Things** — tag tiles for object-recognition labels (animals, vehicles, etc.). Requires ML object tagging on the server.

Clicking a tile filters the grid to assets belonging to that person, place, or tag.

Use the **Refresh** button in the header to reload the Explore data. Sections that return no results from the server are hidden automatically.

---

## Albums

Select **Albums** in the sidebar to open the album landing page.

Three sections are shown:

- **Recent** — recently accessed albums
- **Your albums** — albums you own
- **Shared with you** — albums shared by other users

Click an album tile to open it in the grid view. The album is also added to the sidebar list for quick re-access.

### Creating an Album

Click **Create album** (top right of the Albums page). Enter a name and confirm. The new album appears in the **Your albums** section immediately.

To assign a new or existing album to a watch folder for automatic upload, use **Settings → Watch Folders → album dropdown** on the relevant folder row.

---

## Selection Mode

Click the checkbox icon in the header bar (or use **Esc** to exit) to toggle selection mode.

In selection mode:

- each grid cell shows a checkbox
- clicking a cell toggles its selection state
- a bulk action bar appears at the bottom showing the selection count

**Bulk actions available:**

- **Download** — saves selected remote assets to the configured download folder (local-only assets are skipped)
- **Delete** — permanently deletes selected remote assets from the Immich server after a confirmation dialog
- **Clear** — deselects all items without taking action

Selection mode exits automatically when all items are deselected.

---

## Lightbox and Asset Details

Click any asset in the grid to open it in the lightbox.

- The lightbox shows a preview image. If **Full Resolution Preview** is enabled in Settings → Behavior, it loads the original file instead of a server-generated proxy.
- EXIF metadata is fetched and displayed alongside the asset.
- **Download** saves the original file to the configured download folder.

**Download folder:**

- On the first download, a folder picker opens and the chosen path is saved to `download_target_path` in `config.json`.
- Subsequent downloads go to the same folder without prompting.
- If the target folder is missing at download time, the picker opens again.
- Filename collisions are resolved by appending a numeric suffix (e.g. `photo (1).jpg`).

---

## Album Sync (Bidirectional)

When viewing an album, a footer row shows the linked local watch folder (if any) and two action buttons.

| Button | Action |
| :--- | :--- |
| **Link folder** | Opens a picker to associate a local watch folder with this album |
| **Sync** | Runs a bidirectional sync between the linked folder and the album |

**Sync steps:**

1. Mimick computes SHA-1 checksums for local files in the linked folder.
2. Files present locally but missing from the remote album are uploaded.
3. Assets in the remote album missing from the local folder are downloaded.
4. Name collisions during download are resolved with a numeric suffix.

Album sync is on-demand — it runs only when you press **Sync** and does not run automatically in the background. It respects the same file extension filters as the main sync engine.

---

## Library Settings

These options are in **Settings → Behavior** and apply to the library view.

| Setting | Default | Effect |
| :--- | :--- | :--- |
| **Full-Resolution Preview** | Off | When on, the lightbox loads the original file instead of the ~1440px server-generated preview. Uses more bandwidth. |
| **Thumbnail Memory Cache (MB)** | 80 | RAM cap for decoded thumbnails. Increase to reduce re-fetches when scrolling through large grids. |
| **Download Folder** | Not set | First download opens a folder picker; the chosen path is saved and reused for subsequent downloads. |

See [Performance Tuning](Performance-Tuning) for guidance on choosing values for these settings.

---

## Keyboard Shortcuts

| Key | Action |
| :--- | :--- |
| **F9** | Toggle sidebar |
| **Esc** | Exit selection mode |
