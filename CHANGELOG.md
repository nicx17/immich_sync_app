# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Shift+Click range selection in the library grid. Clicking an asset with Shift held selects the contiguous range from the last-clicked anchor to the current position. Ctrl+Shift adds the range to the existing selection without clearing it.
- Download folder confirmation dialog. When no default download folder is saved, the folder picker now appears on every download. After choosing a folder, a prompt asks whether to save it as the default ("Always") or use it once ("Just Once").
- Download Folder setting in the Library preferences group. Shows the current saved folder with Change and Clear buttons, so users can view and manage the default download location from Settings.
- Batch download summary dialog. After a multi-select download completes, a dialog reports how many assets were downloaded, skipped, or failed, and names the destination folder.
- Unified file conflict dialog for batch downloads. When existing files are found, a single dialog offers Skip, Rename (auto-suffix), or Overwrite, with an "Apply to all remaining conflicts" checkbox instead of spawning one dialog per file.
- Added `cargo test --locked` to `validate.sh` to match CI and catch regressions locally.
- Added detailed permission error dialogs across the Library, Explore, and Server Statistics views to clearly identify missing API scopes (e.g. `HTTP 401/403`) instead of treating them as generic network failures.
- New masonry layout for the Photos view in Library. Images now keep their natural aspect ratios and pack into justified rows instead of a fixed tile grid.
- New `Library Thumbnail Quality` setting with `Auto`, `Thumbnail`, `Preview`, and `Full Size` options. `Auto` chooses a size based on the rendered cell size, and `Full Size` falls back automatically when the server does not provide that image size.

### Changed

- Updated README and in-app missing permission dialogs to explicitly document `asset.statistics` as a required API key scope for fetching server statistics.
- Library layout now uses image dimensions from Immich EXIF data earlier in the load path, so rows are sized more accurately from the first paint.
- Thumbnail memory cache is now sized automatically from available system RAM, using up to 20% of memory with built-in minimum and maximum limits, so the manual memory-cache limit setting has been removed.
- Thumbnail loading now uses separate concurrency limits for smaller thumbnails and larger preview/full-size requests to keep browsing responsive while larger images are still loading.
- Library settings text was shortened and cleaned up to make common options easier to scan.
- Removed the redundant "(Selected via Flatpak portal)" text globally (affects Settings, Diagnostics, Album views) since the app is exclusively distributed via Flatpak.
- `validate.sh` now uses `set -euo pipefail`, passes `--locked` to clippy/test, `--deny warnings` to cargo audit, and fails fast when `cargo-audit` is missing instead of auto-installing it.

### Fixed

- Fixed Shift+Click in the library grid having no effect. The click handler now checks `SHIFT_MASK` and selects the contiguous range between the anchor and clicked position.
- Fixed library downloads silently persisting the chosen folder on first use and never prompting again. The folder picker now appears each time until the user explicitly opts into a default.
- Fixed batch download showing "Download Complete" even when all assets were skipped. The heading now reads "No Assets Downloaded" or "Download Failed" as appropriate.
- Fixed batch download spawning a separate overwrite dialog for every conflicting file simultaneously. Conflicts are now resolved sequentially with a unified dialog.
- Fixed batch download summary showing the full sandbox path instead of just the folder name.
- Selection mode now automatically exits after a batch download completes.
- Fixed an issue where the footer statistics would remain permanently missing if the initial network connection failed at startup.
- Fixed the Server Statistics missing permissions dialog text truncating on narrow screens (like 360px width) by allowing it to wrap.
- Fixed `validate.sh` to automatically apply `cargo fmt` fixes instead of failing on formatting errors.
- Fixed native GTK button hover and scale animations across the app by replacing the rigid CSS transition with a smoother `cubic-bezier` curve.
- Masonry grid scrolling and thumbnail loading are now more stable. Items close to the visible area load first, and the layout no longer shifts as much while images are coming in.
- Large cells can now request higher-quality preview and full-size thumbnail buckets correctly, with automatic fallback when a higher bucket is unavailable on the server.
- Queue Inspector: Fixed the Recent Queue Activity layout so it no longer redundantly duplicates the filename in two columns.
- Queue Inspector: Allowed filenames to smoothly wrap to multiple lines, preventing truncation (cut off with ellipses) when the window is narrow.
- Settings: Replaced the Application Quit button's FlowBox layout with a native Libadwaita ActionRow, fixing an oversized invisible touch target anomaly and matching standard OS styling.

### Refactored

- Decomposed high-complexity functions across the codebase to bring cognitive complexity under 15 and cyclomatic complexity under 8, enforced via `clippy::cognitive_complexity` at warn level. Affected modules: `monitor`, `startup_scan`, `api_client`, `album_sync`, `album_link`, `canvas`, `lightbox`, `queue_manager`, `settings_window`, `main`, and `texture_decode`.
- Extracted `texture_decode` into a directory module with a dedicated `codecs` submodule for JXL, SVG, JP2, and PSD decoders.
- Split JPEG marker parsing (`is_lossless_jpeg`, `find_jpeg_end`) into smaller helpers (`classify_jpeg_marker`, `skip_segment`, `seek_marker`, `scan_sos_entropy`).
- Extracted libraw FFI helpers (`init_libraw`, `configure_libraw_full_decode`, `run_libraw_full_decode`, `unpack_libraw_fallback_thumb`) from monolithic unsafe blocks.
- Split masonry layout packing into `collect_row_indices` and `build_row`; gesture handling into `handle_primary_click`, `handle_secondary_click`, and `toggle_selection`.
- Decomposed `build_settings_window_with_parent` into per-section UI builders and extracted `build_behavior_group` sub-builders (`add_core_rows`, `add_quiet_hours_rows`).
- Split `finalize_upload_progress` into `apply_idle_state`, `apply_active_state`, and `check_batch_notification`.

## [9.6.1] - 2026-05-31

### Changed

- Revised API key permission guidance across the app, README, and wiki to match Immich's exact scope names. The documentation and in-app messaging now call out `user.read`, `albumAsset.create`, and `albumAsset.delete`, and clarify the extra scopes needed for Library / Explore browsing and deletion mirroring.

### Fixed

- Lightbox mouse drag-pan is now smooth and jitter-free when zoomed in. The drag and pinch gestures now share the scrolled-image coordinate space.

## [9.6.0] - 2026-05-25

### Added

- Face visibility filters in the Explore view allowing users to show/hide face-tagged assets with server-side filtering via new `personVisibility` parameter.
- Server Statistics Dialog displaying key library metrics (total assets, photos, videos, archive count, person count, etc.) in a responsive FlowBox layout that adapts to window size.
- Cargo audit configuration to ignore unmaintained EXIF dependencies, streamlining the security audit workflow.
- Subtle press animations across all interactive UI elements (header buttons, sidebar rows, grid tiles, explore/album tiles, settings rows, dialog buttons). Uses a brief scale-down (0.97x, 80ms) on press; animations are automatically disabled when the GTK `gtk-enable-animations` system setting is off.
- RAW rendering mode toggle in Settings > Library: "Full RAW Decoding" switch allows users to choose between fast embedded JPEG preview extraction (~10ms, default) and full sensor demosaic via libraw (~1-3s). The decode cache setting is gated to only appear when full decode is enabled.
- Embedded JPEG preview extraction from RAW files via `libraw_unpack_thumb` + `libraw_dcraw_make_mem_thumb`. Supported across all major RAW formats (CR2, CR3, NEF, ARW, DNG, ORF, RW2, PEF, FFF, RAF, SR2, MRW, etc.) with automatic fallback to full demosaic when no embedded preview exists.
- Toggleable RAW decode cache setting in Settings > Library. Stores demosaiced textures as PNGs on disk to skip re-processing on re-opens. Defaults to off to conserve storage.
- Lightbox swipe gesture navigation: swipe left for next asset, swipe right for previous. Works with both touch and trackpad input. Automatically disabled when zoomed in to avoid conflicting with pan gestures. Essential on mobile viewports (360px) where the prev/next header buttons are hidden.
- XMP sidecar upload support: companion `.xmp` files alongside media are automatically detected and attached via the Immich `sidecarData` multipart field during upload. Searches `photo.ext.xmp` (preferred) then `photo.xmp` (fallback), case-insensitive. Global toggle in Settings > Behavior with per-folder override in Folder Rules.

### Changed

- Mobile and narrow viewport UI overhaul: optimized component dimensions for 360px width screens, adjusted album and explore tile layouts to 2-column on mobile with uniform sizing, reduced header controls footprint, and improved overall narrow-screen usability.
- Album and explore view tile rendering now uses fixed 100px height with viewport-responsive width (via FlowBox homogeneous layout), ensuring consistent thumbnail aspect ratios and eliminating layout jitter on window resizes.
- Grid view minimum columns reduced from 3 to 2 on narrow viewports to fit within 360px width constraints without overflow.
- Added comprehensive rustdoc comments across all source files: module-level `//!` docs expanded to multi-line descriptions, and doc comments added to all public and private structs, enums, functions, and fields.
- Top toolbar redesigned with context-aware search, sorting, and action triggers, providing better integration with face filters and statistics features.
- Explore and album cover layouts now use breakpoint-driven responsive sizing (600sp threshold) to prevent image stretching on different viewport sizes.
- Server statistics API endpoints enhanced to support library metrics and aggregation queries.

### Fixed

- Lightbox now shows a video poster with a clickable play badge when navigating to a video asset (via arrow keys or Prev/Next), instead of the "Preview unavailable" overlay. Clicking the badge hands off to the same external-player flow used by grid clicks (local files via system default app, remote files downloaded to `~/.cache/mimick/video/`). Zoom controls, resolution toggle, and download button are hidden while a video is the current asset.
- RAW embedded preview extraction now scans the file directly for the largest SOI-bounded JPEG as the primary path, instead of relying on libraw's preferred thumbnail. Resolves the postage-stamp preview problem on cameras that store a tiny JPEG followed by a full-resolution one (Sony ARW and similar). When no JPEG is present in the file at all (Samsung DNG, OnePlus DNG, and other phone DNGs that store their preview as uncompressed TIFF strips), the bitmap thumbnail from `libraw_dcraw_make_mem_thumb` is used as the fallback with container orientation applied. The full RAW demosaic path is unchanged.
- Embedded JPEG previews extracted from RAW files (.fff, .pef, .dng, .3fr, .RAW, etc.) now display with correct orientation. The extractor routes through `Pixbuf::apply_embedded_orientation` for JPEG thumbnails and applies the RAW container's `flip` field via `Pixbuf::rotate_simple` for bitmap thumbnails. Previously `Texture::from_bytes` ignored EXIF orientation tags.
- RAW thumbnails from cameras that pre-rotate the embedded preview (Kodak DCR, Nikon E5700 NEF, Leaf MOS) no longer appear double-rotated. A new `is_thumbnail_prerotated` check compares the preview aspect ratio against rotated sensor dimensions and skips the container flip when the preview is already oriented correctly.
- SOI JPEG scanner no longer selects lossless JPEG payloads (SOF3 / `FF C3`) as preview candidates. RAW files wrap compressed Bayer sensor data in SOF3 containers that are never renderable. Previously the scanner selected these multi-MB blobs as the "largest JPEG", wasting a Pixbuf decode attempt before falling back to libraw. Affects 19 of 55 tested RAW formats (Nikon NEF/NRW, Pentax PEF, Canon CR3, Kodak KDC, Panasonic RW2/RWL, Leica DNG, Hasselblad 3FR, Sony SRF, Phase One CAP, Sigma X3F).
- Local folder thumbnail generation for RAW files now skips the `gdk_pixbuf::from_file_at_scale` attempt entirely, going straight to the custom RAW decoder. Pixbuf never recognises RAW formats, so the previous attempt always failed with "Couldn't recognize the image file format" before falling through to the custom decoder, adding latency and log noise on every RAW thumbnail.
- Lightbox details pane toggle button is no longer hidden at narrow viewports (360px). Previously all three header buttons (prev, next, details) were hidden when the split view collapsed, leaving no discoverable way to access the EXIF details pane on mobile.
- Lightbox trackpad pinch-to-zoom now works reliably and is smooth. Zoom is now jitter-free: scroll adjustment ranges are pre-set synchronously to match the new content size so the scroll position applies in the same frame as the resize, eliminating the one-frame flicker from the previous deferred update.
- Album/explore tile thumbnails now maintain uniform sizes within a viewport and display correctly at 360px window width, with no collapse on window-height changes.
- FlowBox children per line constraints adjusted (min 2, max 6 for albums) to ensure 2-column layout on mobile while limiting tile width growth on desktop.
- Raw asset handling and EXIF metadata extraction now properly exports `gexiv2_metadata_free` for modern gexiv2 linking compatibility.
- Lightbox JPEG XL decoding switched from `jpegxl-rs` (system `libjxl` bindings, frequently mismatched at runtime) to `jxl-oxide` (pure-Rust). Removes the "Generic Error. Please build `libjxl` from source" failure mode on valid JXL files and drops the libjxl system dependency. Now also handles 1-channel grayscale and 2-channel grayscale-with-alpha JXL inputs by expanding to RGB / RGBA.
- Lightbox RAW decoding now uses `libraw` (vendored C++) as the primary decoder with `imagepipe` as a pure-Rust fallback. libraw applies per-camera color matrices and white balance so output matches darktable / Lightroom instead of imagepipe's generic sRGB pipeline. Adds proper support for ARI (Arriraw), FFF (Hasselblad), CR3 (Canon mirrorless), ERF (Epson), Sony ARW variants, Phase One IIQ, and Sigma Foveon X3F -- formats imagepipe either rejected outright or rendered with wrong colours.
- libraw decoder now explicitly enables `use_camera_wb` (camera as-shot white balance) and `user_qual = 3` (AHD demosaic) via direct `libraw-sys` FFI, since the `libraw-rs` Rust wrapper does not expose those params. Fixes the muted / blue cast that was visible on Nikon NEF, Hasselblad FFF, and Canon CR3 files compared to darktable.
- Added PSD (Photoshop) lightbox decoding via the pure-Rust `psd` crate. Previously `.psd` files fell through every decoder and showed "Preview unavailable".
- Fixed `gtk_gesture_group: assertion ... == ... failed` GTK-CRITICAL that fired every time the lightbox opened. The drag gesture must be attached to its widget _before_ `group_with(&pinch)` is called.
- Release profile switched from `panic = "abort"` to `panic = "unwind"` so `catch_unwind` actually catches third-party decoder panics. Previously, `rawloader`'s out-of-bounds slice access on certain malformed RAW files (IIQ, ARW, CR3) aborted the entire app -- `catch_unwind` is a no-op under abort. The ~60-100 KB unwinding-machinery cost is worth the crash resistance for a viewer that ingests user files.
- Lightbox SVG rendering now uses `resvg` (pure-Rust) instead of relying on the gdk-pixbuf SVG loader, which is not always present in Flatpak runtimes. Restores SVG previews regardless of which pixbuf loaders the host bundles.
- Lightbox SVG parser now tolerates real-world SVGs that use undeclared XML namespace prefixes (e.g. `c2pa:` C2PA provenance metadata from Adobe exports). Missing namespaces are injected with placeholder URIs before parsing.
- Lightbox JPEG decoding now uses `libjpeg-turbo` (`turbojpeg` crate, vendored) as the primary decoder. Supports arithmetic-coded JPEGs (DAC marker) that `image-rs` and `gdk-pixbuf`'s Glycin backend reject with "header DAC is not supported".
- Lightbox WebP decoding now uses `libwebp` (`webp` crate, vendored) as the primary decoder. Handles complex WebP bitstreams that `image-rs`'s pure-Rust WebP decoder rejects as "corrupt".
- Lightbox JPEG 2000 (`.jp2`) decoding added via `openjpeg` (`jpeg2k` crate, vendored). Previously these were silently unsupported by both pixbuf and image-rs and only showed "Preview unavailable".
- Lightbox RAW decoder now catches panics from `imagepipe`/`rawloader` (out-of-bounds in `crw.rs` and `packed.rs` decoders on certain RAW files) and falls through to libraw rather than crashing the worker thread.
- Process-wide panic hook now filters out the noisy stderr messages from caught `rawloader`/`imagepipe` panics so the log stays readable when iterating over a folder of RAW files.
- Lightbox navigation between assets is now race-free: each load tags itself with a generation counter and skips its own `set_paintable` if the user has already navigated to a different asset by the time the decode finishes. Previously a slow RAW decode could clobber a newer image after rapid Next/Prev.
- Lightbox slide transitions now fire after the new image decodes, not before. Previously the slide animated to a copy of the current image (or to a blank target) and the new image popped in after; now the current image stays visible with a loader spinner while decode runs, and the slide animates straight to the loaded result.
- Pixbuf and image-rs decode failures in the lightbox now log at debug level instead of silently returning, making future format-support issues diagnosable from `mimick.log`.
- Dropped MPO from the supported-extensions registry; Immich's server rejects the format, so mimick no longer queues `.mpo` files for upload or tries to render them in the lightbox.
- Centralized the RAW-extension list in `media_kinds` (next to MIME mappings) and removed the duplicate hardcoded list from `library/mod.rs`. New `media_kinds::is_raw_ext()` helper for callers.
- Local folder thumbnails now generate correctly for RAW images (CR3, NEF, ARW, DNG, ORF, etc.) and other formats without built-in gdk-pixbuf loaders (HEIF, JXL, JP2, PSD). When `Pixbuf::from_file_at_scale` fails, the thumbnail cache falls back to `load_texture_blocking` (the same decoder pipeline used by the lightbox) and converts the result via `TextureDownloader` (GDK 4.10+) for scaling and disk caching. Previously these assets showed as broken/missing thumbnails in the linked folder local view.
- Explore view no longer filters the people row down to a single person after navigating away and back. The shared `search_entry` widget previously carried a person's name set by the person-click handler into the next Explore-view activation, where it was reapplied as the people filter. Clearing the entry on `content_stack` view switches restores the full people list. The same fix applies to the Albums view.
- Lightbox slide animation between assets is smoother. The stack child switch is now deferred by one idle tick so the target `GtkPicture` re-measures with the newly-painted texture before the slide starts; the first frame no longer captures the previous texture's aspect ratio.
- Added debug-level logging when the "Show hidden" people filter is toggled, printing the new state and the number of hidden people in the cached list so wiring issues vs empty server-side data are distinguishable from the log.

### Security

- Replaced `std::env::temp_dir()` usage in the RAW decode cache with `tempfile::NamedTempFile` to avoid predictable temp path vulnerabilities (`rust.lang.security.temp-dir.temp-dir`).

## [9.5.4] - 2026-05-17

### Added

- Per-folder sync rules: each watch folder can be set independently to **Upload only**, **Download only**, or **Full sync**. Bidirectional sync deletions (local→remote, remote→local) are configurable per folder and independent of the sync method, with both off by default (#120).
- Periodic remote-album reconciler (5 min) so changes made directly on Immich mirror back without waiting for the next startup scan (#120).
- Pre-flight checksum dedup before upload (batched on startup, per-asset on live events) and local rename detection so renaming doesn't cause an upload+trash cycle (#120).
- Three-tier local trash (`gio` → XDG portal → manual XDG) for reliable trashing of Flatpak document-portal paths (#120).
- Documented Immich API key permission requirements end-to-end: README/wiki tables, in-app welcome and field subtitles, 401/403 error guidance, and a troubleshooting entry (#120).
- Asset generator tool overhaul and testing wiki page (#118); Codacy badge in the README (#116).

### Changed

- Split three large modules into directory modules with focused submodules to improve incremental compile times: `library/mod.rs` (3725 LOC → 7 submodules), `api_client.rs` (2224 LOC → 6), `settings_window.rs` (2173 LOC → 2). No behaviour change (#117).
- Album→folder local trash is gated off until the upstream Flatpak portal trash bug is fixed; the settings toggle is disabled with an explanatory subtitle (#120).
- Info logs are quieter — internal mechanics moved to debug; `MIMICK_PROFILE=dev` defaults to `mimick=debug` (#120).
- CI: switched workflows to `ubuntu-latest`, decoupled the two Flatpak manifests' build triggers, removed the deprecated maintainer-approval workflow, and CodeQL now sets up Rust + `cargo fetch` for reliable extraction.
- Dependabot: bumped `taiki-e/install-action`, `github/codeql-action`, `flatpak/flatpak-github-actions`, and `release-drafter/release-drafter` (#119).

### Fixed

- Live file-monitor events no longer re-upload already-synced files. The `MonitorEvent::Ready` handler now consults `ShardedSyncIndex` via `sync_decision` before queueing, mirroring the startup-scan path. Without this, `PollWatcher` (Flatpak's polling watcher for portal FUSE paths) could lose its internal "seen" state under heavy I/O and re-emit Create events for unchanged files.
- Multiple deletion-safety fixes in the new sync engine: strict `album_id` match so re-targeting a folder doesn't mass-trash local files; per-album reconcile lock so manual + periodic sync can't race; two-tick confirmation for batch deletions >5; assets present elsewhere on the server (in another album, or referenced from another watch folder) are kept or only de-associated rather than fully trashed; orphan classification requires physical absence on disk (#120).
- Immich base64 asset checksums normalised to hex at deserialisation; self-induced filesystem events suppressed so own trash/download no longer propagate back as sync work; live monitor clears sync records only after the server op succeeds so transient failures retry on the next tick (#120).

## [9.5.3] - 2026-05-11

### Added

- Back navigation in the library window. The header bar now includes a back button (Alt+Left) that returns to the previously visited view (Photos, Album, Explore, etc.). A nav-history stack is maintained per library source — searches are not pushed since they're ephemeral, and consecutive duplicates are coalesced. The button is disabled when there's no history to return to.
- Lightbox image zoom support via Ctrl+scroll wheel, trackpad pinch gesture, on-screen `−` / `+` / `100%` button group, and Ctrl+`+` / Ctrl+`-` / Ctrl+`0` keyboard shortcuts. The current zoom percentage is shown in the centre button (click it to reset to fit). The zoomed image scrolls within the viewer for panning. Zoom level resets to fit-to-window when navigating between assets.
- Lightbox slide animation when navigating between images. Forward navigation (next button or Right arrow) slides the new image in from the right; backward navigation slides it in from the left. Falls back gracefully when GTK animations are disabled in system settings.
- Configurable test-asset generator script for reproducing deduplication and startup scan benchmarks across all supported API asset formats (#101).
- Graceful shutdown for in-flight uploads. Quitting the app now stops accepting new tasks, waits up to 5 seconds for active uploads to finish cleanly, then cancels anything still in flight via a cancellation token. Uploads cancelled at the deadline are persisted to the retry queue and resumed on the next launch instead of leaving partial assets on the server.
- Profile switcher for development and testing via the `MIMICK_PROFILE` environment variable. Each named profile (`MIMICK_PROFILE=dev`) gets fully isolated state: its own config file, sync index, retry queue, thumbnail cache, and keyring entry. The GTK application id is varied per profile so multiple profiles can run simultaneously without activating each other's windows. Portal folder grants are shared across profiles since they are scoped to the Flatpak app-id.

### Changed

- Disk thumbnail cache is now pruned on a 10-minute background interval instead of only at startup, and its cap was raised from 500 MB to 1 GB so long sessions no longer grow unbounded.
- Consolidated the three previously hand-maintained extension/MIME tables (file watcher, upload client, library enumerator) into a single `phf` perfect-hash registry. Eliminates table drift and turns the watcher's hot-path extension check from a 68-entry linear scan into an O(1) lookup.
- Hardened security by adding strict path sanitization against directory traversal on downloads, and enforcing `http`/`https` scheme validation for server URLs (#98).
- Centralized read/write lock configuration access into `AppContext`, eliminating redundant disk parsing and streamlining context usage across modules (#98).
- Replaced standard non-async locks (Mutex/RwLock) with `parking_lot` for faster operations, and added `atomic_write()` for crash-safe file persistence (#98).
- Eliminated single-lock bottlenecks by replacing the monolithic `SyncIndex` with a 16-shard `ShardedSyncIndex` backed by `RwLock` shards (#99).
- Significantly improved startup scan speeds by rewriting it as a two-stage parallel pipeline using `rayon` for fast directory enumeration and bounded asynchronous queueing (#99).
- Added a worker pool cap to file monitor events with backpressure channels rather than unbounded blocking spawns (#99).
- Implemented a custom `LibraryAssetModel` replacing the monolithic `gio::ListStore` to enable client-side sorting independently of server retrieval (#100).
- Memory unbounded growth issues when scrolling large remote libraries are resolved using a new 400-asset sliding window with FIFO eviction (#100).
- Implemented background lookahead thumbnail prefetching and lowered scroll trigger thresholds so thumbnails pop in faster while dragging down the grid (#100).
- Added formal sorting support (`SortOrder`) across metadata and OCR search endpoints from the client side (#100).
- Relocated the server connection status indicator from the sidebar top to the bottom for better visibility and layout balance.
- Removed the redundant checkbox toggle icon from the library header bar; selection mode is now managed via Ctrl-hold or keyboard interaction.
- Removed support for `.xmp` files from the media scanning and MIME detection logic as they are not utilized by the application.
- Removed the "Sync State" option from the library sort dropdown; it was only meaningful in unified/local views and produced no useful ordering in the standard photos view.
- Transfer direction in the progress bar is now shown as an icon (`mimick-upload-symbolic` / `mimick-download-symbolic`) rather than the text prefix "Uploading" / "Downloading", keeping the label focused on the active filename and speed.

### Changed

- Explore page Places section now shows all cities with geotagged assets instead of only the small popular subset returned by `/api/search/explore`. A new `fetch_all_places` method pages through all assets with EXIF city data, collecting unique cities and a representative thumbnail asset per city.

### Fixed

- Details pane timestamps (Taken and Created) no longer display the raw UTC ISO 8601 string with a confusing `+00:00` suffix. A new `format_datetime_display` helper parses the timestamp and formats it as `YYYY-MM-DD HH:MM:SS`, stripping the offset indicator so the displayed time matches the unambiguous value Immich recorded.

- Search pagination — particularly OCR search — now uses Immich's `nextPage` field as the source of truth instead of a "did we get a full page?" heuristic. Previously, when Immich's search response post-filtered results (for visibility, archive, library scope) it returned short pages even with more matches available, causing pagination to stop early and hide the rest. Applies to all four search endpoints (Smart, OCR, Metadata, Advanced) and to album/unified variants that route through the same endpoint.
- Closing the settings window no longer also closes the library window when background sync is disabled. The "quit when settings closes" path now checks for other open application windows first and only quits when settings was the only window left.
- Settings: the "Enable Library View" toggle, "Open Originals in Lightbox" toggle, and "Thumbnail Memory Cache (MB)" spinner now save immediately on change instead of requiring a click on the connection-save button. Each handler skips the disk write when the value matches the existing config (avoids redundant writes during initial population).
- Settings: the connection-save button has been renamed from "Save Connection Settings" to "Save Credentials" so the action label matches what the field group is actually about (URL + API key).
- Concurrent workers racing to create the same album on first run now serialize via a per-album-name lock with double-checked cache lookup, preventing N duplicate albums being created simultaneously (one per queued file).
- `fetch_all_albums` now collapses concurrent callers into a single network request via a fetch lock with double-checked `albums_fetched` flag. Previously, concurrent startup-scan workers each fired an independent `GET /api/albums`, then re-inserted the same entries and reported every album as a false-positive duplicate.
- Duplicate album detection now compares IDs within a single server response (built into a fresh map before replacing the cache), so "same name, same ID" entries from a re-fetch are ignored silently while genuine server-side duplicates (same name, different ID) still warn and keep the first.
- `refresh_album_cache` now holds the fetch lock across its clear → reset → refetch sequence, closing a race where a concurrent in-flight fetch could populate the cache after the clear but before the reset, leaving the cache empty with the flag set to true.
- Sync index state is now flushed to disk when the upload queue goes idle, ensuring a clean restart never re-queues already-uploaded files even after an immediate exit following a fast batch.
- Sync index is now flushed on graceful shutdown (tray quit, window close) so records written since the last 10-second periodic flush are not lost.
- SIGINT and SIGTERM signals now route through the GTK graceful shutdown path instead of killing the process immediately, so uploads drain, retries persist, and the sync index flushes before exit.
- `flush()` in the sync index now clears dirty bits only after a successful write. Previously, a failed `atomic_write` (e.g. disk full) would silently mark shards clean, causing subsequent flush calls to skip them and permanently lose the unwritten records.
- Removed a broken auto-flush trigger in `record_synced` that fired when `shard.entries.len()` was a multiple of 50 — this checked total entry count, not dirty-since-last-flush count, so it fired unpredictably and never for small datasets. Flush scheduling is now handled solely by the 10-second timer and idle-queue flush.
- Concurrent `check_connection` callers (queue workers, library ping loop, main loop) now serialize via a single-flight lock. Callers that arrive while a probe is already in flight coalesce onto its result within a 1-second window, eliminating redundant LAN/WAN ping bursts at startup without suppressing the 5-second library re-check.
- Escape key and Clear button now properly exit selection mode entirely instead of just clearing selected items.
- Asset sync status in the lightbox details pane and thumbnail hover now reflects the asset's true state (local only, remote only, or both) instead of being implied by the active view. Root cause: Immich returns SHA-1 checksums as base64 in API responses while the sync index stores them as lowercase hex; the representations are now normalised on read so checksum-to-path reverse lookups resolve correctly across all views including unified and album-unified.
- In-app symbolic icons (`mimick-upload-symbolic`, `mimick-download-symbolic`) are now compiled into the binary via `glib-build-tools::compile_resources` and registered at startup with `gtk::IconTheme::add_resource_path("/dev/nicx/mimick/icons")`, replacing a `theme.add_search_path(env!("CARGO_MANIFEST_DIR"))` call that resolved to a nonexistent path in all non-`cargo run` environments.

---

## [9.5.2] - 2026-05-07

### Added

- Right-click context menu on grid assets with download, open-with, and delete actions gated by asset type and source.
- Sync status icons (cloud/computer/check) and centered video badge overlay on grid thumbnails, replacing the old generic icon.
- Auto-refresh library surfaces after album creation, bulk delete, download completion, and album sync mutations.
- Queue inspector launchable from the library header bar, with enlarged dialog and improved long-path rendering.
- Settings save acknowledgement dialog on explicit save.

### Changed

- Updated `h2` and `tower-http` dependencies and removed unused `iri-string` dependency.
- Bumped `github-actions` dependencies (`taiki-e/install-action` and `github/codeql-action`).

### Fixed

- Sync index is now stored in the persistent data directory instead of the cache directory. Clearing the app cache no longer wipes sync state or triggers a full re-upload. Existing index files are migrated automatically on first run.
- Ctrl+click selection no longer skips the first asset. Holding Ctrl now reveals checkboxes transiently; clicking an asset commits the selection. Releasing Ctrl without clicking dismisses selection mode.
- Local and unified views for linked albums are now scoped to the selected album's linked folder only, instead of showing assets from all linked albums.
- EXIF orientation is now applied to thumbnails and lightbox images via `apply_embedded_orientation()`. Stale thumbnail cache entries are invalidated with a new cache key version.
- Details pane is now fixed at 320px width with `max_width_chars` constraints on labels so it no longer resizes dynamically with image dimensions.
- Grid view thumbnails now use a fixed 356px width (16:9 at 200px height) instead of expanding to fill available space.
- Explore page places grid and album tab tiles are now sized at 300x220px with a horizontal meta row layout (name left, count right) matching the explore page grid style.
- Consolidated duplicate refresh buttons by removing the sidebar refresh button and keeping the status bar one. Added F5 keyboard shortcut for refresh.

---

## [9.5.1] - 2026-05-04

### Fixed

- Startup view no longer forced to Settings when background sync is disabled; the default window (Library if enabled) now opens correctly.
- Direct navigation between Album and Explore views now works without routing through Photos first.
- Default folder name albums now auto-link to the correct remote album and no longer create duplicate config entries on re-save.
- Folder-linking UI is now hidden when navigating to Explore or Albums from within an album view so it only appears in relevant contexts.
- Local asset count in album sync no longer double-counts files reached via symlinks or duplicate paths.

### Changed

- Removed `--env=MESA_LOG_LEVEL=error` from Flatpak manifests

---

## [9.5.0] - 2026-05-03

### Added

- New Library View feature for browsing assets in-app with dedicated library navigation and interactions.
- Foundational library module components were introduced, including shared state wiring through `AppContext` and a dedicated library surface.
- Library search expanded with metadata filters, timeline search, OCR-selectable search dimensions, and random asset search.
- New Explore surface added for people, places, and things with integrated fetch flows.
- Library sidebar now includes album-focused navigation and source-aware controls for browsing.
- Local source enumeration was added for library integration and settings-driven source visibility.
- Asset details now include EXIF model support and richer asset-detail fetching.
- Albums view added with Recent, Owned, and Shared sections, plus in-app album creation.
- Album synchronization support added for library workflows.
- Thumbnail cache now supports ThumbHash + Base64 data paths for faster preview rendering.

### Changed

- Library source switching logic was refactored for readability and cleaner state transitions.
- Explore view and sidebar composition were refactored to simplify state management and UI behavior.
- Thumbnail loading/caching was reworked to reduce redundant fetches and improve cache initialization behavior.
- Thumbnail decode/load paths now use cancellation-aware, bounded work to smooth scrolling under heavy grids.
- Streaming downloads, disk-cache pruning, and sync-condition checks were optimized for runtime efficiency.
- App identifier references were renamed from `io.github.nicx17.mimick` to `dev.nicx.mimick` across desktop and project configuration.
- Documentation was updated to cover library view behavior and permissions in current builds.

### Fixed

- Grid view CSS states and layout behavior were polished for more consistent library interactions.
- Asset download error handling in library-related fetch paths was improved.

## [9.4.2] - 2026-04-28

### Changed

- Revised app logic to background sync. Now if background sync is disabled app won't persist in background when closed using window close button. App startup will open the settings window for the same. Quit action will work the same (Close the app).

- UI: about section is now moved to title bar

## [9.4.1] - 2026-04-25

### Added

- Stream startup scan candidates directly into the upload queue to reduce startup memory churn.

### Changed

- Fetch Immich albums immediately after saving connection settings.
- Parse album list responses into typed structs instead of generic JSON for safer handling.
- Use local time for quiet-hours checks.
- Reduce cloning of folder status state during settings UI refresh and avoid cloning the sync index during persistence to improve performance.
- Increase settings window startup width while preserving the 360px minimum mobile layout target.

### Fixed

- Fix live watcher queue metadata after settings changes so queue state remains consistent.
- Prevent the settings window from auto-saving partially populated UI state.
- Avoid creating albums during startup scan inspection and ensure album creation only happens when appropriate.
- Fix Application action buttons so About and Quit keep matching visual and touch target sizing.
- Initialize the Pause button label from the real queue state on first render.
- Fix install method card overflow so cards remain within the install block width.

### Removed

- Removed unused legacy config fields and dead monitor code.
- Removed the grain/noise background effect from the site theme.

## [9.4.0] - 2026-04-23

### Added

- Settings: Live auto-apply for most preferences (workers, quiet hours, folder rules, per-folder album selection, watch-folders). Connectivity fields (API key and server URLs) are now applied only when explicitly saved from the Connectivity section.
- Single-batch sync summary notification: multiple concurrent upload workers now aggregate results and emit a single "processed" summary notification when a sync batch completes.
- Flatpak packaging now targets the GNOME 50 runtime for current desktop compatibility.
- GitHub Pages repository pipeline now dynamically generates a `mimick.flatpakref` file for one-click graphical installations.

### Changed

- Logging: Console output is colorized by level and file logs use a plain, machine-friendly formatter with automatic rotation (approx. 2 MB per file, keep 5). See README and wiki for configuration details.

## [9.3.0] - 2026-04-14

### Added

- New notification toggle. Allow user to enable or disable the notifications sent through the app

### Changed

- Replaced `secret-tool` (libsecret CLI) with the `oo7` Rust crate for credential storage. Inside Flatpak, credentials are now stored in a portal-encrypted file within the sandbox. On native installs, the desktop's D-Bus Secret Service (GNOME Keyring, KWallet) is used directly. This eliminates the `user interaction failed` error that occurred when `secret-tool` tried to render a prompter dialog across the Flatpak sandbox boundary.
- The `.flatpakrepo` file now includes a `RuntimeRepo` directive pointing to Flathub. This allows Flatpak to automatically resolve and download the required GNOME Platform runtime on systems where Flathub is not pre-configured (notably Ubuntu 25+ and certain Fedora spins).
- Removed `libsecret` / `libsecret-1-dev` from build prerequisites. The `oo7` crate is pure Rust and requires no system-level keyring library at build time.
- Removed hardcoded `GSK_RENDERER=gl` from Flatpak manifests. GTK4 now auto-detects the best renderer (Vulkan, NGL, GL) for the host GPU.
- Removed unnecessary `--talk-name=org.freedesktop.secrets` from Flatpak manifests. The `oo7` crate uses the Secret portal inside the sandbox and does not need direct D-Bus access to the host keyring.
- Added `MESA_LOG_LEVEL=error` to Flatpak manifests to suppress harmless Mesa driver developer warnings (FINISHME notes) from cluttering application logs.
- Consolidated all documentation from `docs/` into the project wiki. The `docs/` markdown files have been removed to prevent drift.

### Fixed

- Fixed duplicate URL toggle validation handlers that caused two error dialogs to appear when disabling the last enabled URL switch.
- Added missing config fields (`startup_catchup_mode`, `upload_concurrency`, `quiet_hours_start`, `quiet_hours_end`) to the diagnostics redacted export and plain-text summary.
- Fixed Flatpak installation failing with `org.gnome.Platform was not found` on fresh Ubuntu and Fedora installations that do not ship Flathub enabled by default.

## [9.2.0] - 2026-04-14

### Changed

- Added support for`SingleMainWindow=true` to the `.desktop` launcher to better integrate with GNOME 50+ dock contexts preventing redundant "New Window" options.
- Migrated desktop notifications from the `notify-send` system command to native `gio::Notification`, fixing the issue where notifications would silently fail inside the Flatpak sandbox.
- Notifications now correctly display the app's SVG icon natively via the XDG Notification Portal constraint.
- Removed redundant async blocking wrappers (`tokio::task::spawn_blocking`) around notification dispatches, delegating instead directly to `glib::idle_add_once`.
- Cleaned up redundant branch metadata keys in the Flatpak manifests to optimize build parsing.
- Refined the "Startup Catch-up Mode" drop-down label length in the Settings UI so it no longer visually clips within its ComboRow box.

## [9.1.1] - 2026-04-10

### Changed

- Switched Flatpak repository to correctly advertise as `stable` instead of defaulting to a `beta` channel badge.
- Reconfigured the Flatpak repository builder workflow (`flatpak-repo.yml`) to exclusively deploy on new tag releases rather than on every push to the `main` branch.

### Fixed

- Fixed an internal GTK critical focus assertion error (`box != NULL`) that occurred when opening the folder rules configuration dialog.
- Fixed a bug where a discarded failed-upload task could leave a persistent "Pending: 1" ghost label on the folder configuration UI across application restarts.

## [9.1.0] - 2026-04-08

### Added

- Expanded supported media formats to match latest Immich server: AVIF, BMP, HEIF, JPEG 2000, JPEG XL, PSD, SVG, 3GPP, AVI, FLV, M4V, Matroska (MKV), MP2T, MXF, and more. The app now recognizes and uploads all Immich-compatible image and video extensions.

### Changed

- The settings window now explicitly follows the desktop light/dark appearance preference at startup, allowing light mode when the system theme is light.
- The `Status` page now uses a standard symbolic page icon for more consistent rendering across icon themes.
- Flatpak packaging now targets the GNOME 50 runtime for current desktop compatibility.

## [9.0.0] - 2026-03-29

### Fixed

- Fixed an Immich asset timestamp regression where newly uploaded files could land at the wrong timeline time or lose their intended timezone after server-side metadata processing.

### Changed

- Upload metadata handling now preserves filesystem-based creation times more reliably and reapplies the local timezone after upload so Immich keeps the correct asset date placement.
- The settings window now uses `Status` and `Settings` pages, shows the first-run API-key guidance at the top of the configuration flow, and no longer forces dark mode.
- `Save & Restart` has been replaced with live `Save Changes` behavior that updates the running API client, queue policy, upload worker count, and watched folders without relaunching Mimick.
- Watch-folder changes now reconfigure the live filesystem monitor in place, so adding or removing folders takes effect immediately after saving.

## [8.0.0] - 2026-03-25

### Added

- **Health Dashboard**: A visual status area on the Controls page showing active server route, watched folder count, pending items, recent retries, and latest errors.
- **Per-Folder Status**: The settings UI now displays the pending queue count and last sync time specific to each configured watch folder.
- **Permission Health Checks**: On startup, Mimick now verifies that it still has read access to all configured directories. If a Flatpak permission is lost, a warning is prominently displayed.
- **Safer Startup Catch-Up Controls**: Added a "Startup Catch-up Mode" dropdown in settings allowing users to limit startup scans to "Recent Changed Only (7 days)" or "New Files Only" to save on disk I/O.
- **Actionable Errors**: Meaningful connection failure and folder access loss messages replace generic request timeouts.
- **Better Album Picker**: The per-folder album selector is now a modal search dialog. Users can filter existing Immich albums by name, pick the default folder-name behavior, or type a new name to create an album on the fly.
- **First-Run Wizard**: When no API key is configured, Mimick automatically opens the Setup page and displays a welcome banner. The "Save & Restart" button is disabled until an API key is entered, preventing silent broken-connection states.
- **Notifications That Matter**: Replaced per-file notification spam with a single batch summary notification that fires once a sync cycle completes. Added a dedicated "Connection Lost" notification that fires after consecutive failures.
- **Upload Concurrency**: Users can now configure between 1 and 10 parallel upload workers in the settings, allowing for better tuning based on network capacity.
- **Quiet Hours**: Added a configurable quiet-hours window to pause uploads during specific hours of the day (e.g., to prevent impact on nighttime network usage).
- **Mobile Responsive UI**: Refactored the settings window from a rigid `adw::ApplicationWindow` to a native `adw::PreferencesWindow`. Primary controls and action buttons now use adaptive `FlowBox` layouts that auto-stack vertically on narrow displays (down to 360px), ensuring the app is fully usable on Linux phones and small monitors.
- **Adaptive Folder Rows**: Watch folder entries now use `adw::ExpanderRow` to hide additional settings (Album, Rules, Remove) until clicked, maximizing screen space on mobile.

### Fixed

- Fixed an "endless loop" bug where offline network conditions caused already-synced files to be incorrectly re-queued for reassociation.
- Fixed an issue where the processed file count in the UI would increment infinitely during network failures.
- Fixed a bug where a previously selected album target reverted visually to a "Custom Album" field after an application restart.

## [7.0.0] - 2026-03-22

### Added

- A queue inspector in the settings window with recent queue activity, failed-item visibility, per-item retry actions, `Retry All Failed`, and `Clear Failed Queue`.
- Manual sync controls in both the settings window and tray menu with `Pause / Resume` and `Sync Now` actions.
- Per-folder sync rules for ignoring hidden files, limiting maximum file size, and restricting allowed file extensions.
- A diagnostics export bundle that writes a support-friendly snapshot containing a summary, config copy, status cache, retry queue, sync index, and log file without including the API key.
- Best-effort environment-aware pausing for metered-network and battery-power operation.

### Changed

- Startup scans and live monitoring now apply the same per-folder rule checks and temporary-file filtering before queueing uploads.
- Shared runtime state now records recent queue events, pause reasons, the last completed file, and diagnostics export counts for better visibility and supportability.
- The settings window now separates `Setup` and `Controls`, uses a slimmer layout, and keeps `Close`, `Quit`, and `Save & Restart` pinned in a footer.
- Documentation now covers the new sync controls, diagnostics workflow, per-folder rules, and current test/packaging flow.
- CI and Flatpak publishing documentation now match the current `cargo fmt`, `cargo clippy --locked`, `cargo test --locked`, and containerized Flatpak build setup.

## [6.0.0] - 2026-03-15

### Added

- A startup catch-up scan that walks watched folders on launch and queues media that was missed while Mimick was not running.
- A local sync index that records previously synced files so unchanged media can be skipped quickly on later startups.

### Changed

- Changing the target Immich album for a watched folder now causes unchanged files to be reassociated to the new album on a later startup instead of being ignored.
- If a previously targeted album no longer exists, Mimick now refreshes album resolution and retries with the current configured album target.
- Terminal and file logs now include timestamped detailed formatting for easier troubleshooting.
- Flatpak tray integration now uses a narrower StatusNotifier permission model and no longer requests broad `org.kde.*` bus-name ownership.

## [5.0.1] - 2026-03-14

### Added

- GitHub releases now attach a signed `mimick.flatpakrepo` file and a `SHA256SUMS.txt` checksum file for easier end-user installs.

### Changed

- The GitHub Pages Flatpak repository workflow now signs published repo metadata with a dedicated GPG key and embeds the public key in the generated `.flatpakrepo` file.
- The release workflow now uses the same Flatpak signing key material from GitHub Actions secrets so release assets match the published repository trust chain.

## [5.0.0] - 2026-03-14

### Added

- A built-in **Run on Startup** setting that requests desktop-portal background permission in Flatpak builds and writes a native autostart desktop entry outside Flatpak.
- Friendly folder labels for portal-backed watch directories, so selected Flatpak folders show names like `Screenshots` instead of raw `/run/user/.../doc/...` paths.
- Real **Save & Restart** behavior that relaunches Mimick after settings are saved.
- Explicit **Close** and **Quit** actions in the settings window, plus a launcher **Quit Mimick** desktop action.
- A published GitHub Pages landing page for the Flatpak repository with direct install instructions and repository links.

### Changed

- Flatpak builds now use selected-folder access through the file chooser portal instead of `--filesystem=home`.
- Folder monitoring inside Flatpak now uses a polling watcher so portal-backed directories continue to sync reliably.
- Local Flatpak development builds now use the same selected-folder permission model as the deployed app.
- App quit paths now shut down gracefully instead of using a hard process exit from the tray.
- The Flatpak repository landing page has been redesigned with a simpler, more cohesive visual style and a one-click copy action for install commands.

## [4.0.0] - 2026-03-14

### Changed

- Added Flatpak packaging support
- Removed default photo watch path configuration on startup
- Polished AppStream metadata for Flathub compliance

## [3.0.0] - 2026-03-09

### Added

- **Complete Rust Port**: Entire application rewritten from Python/PySide6 to Rust + GTK4 + Libadwaita. Binary drops from ~80MB (PyInstaller bundle) to ~2MB.
- **Tokio async runtime**: Concurrent upload workers (configurable, default 3) with streaming `reqwest` multipart — constant RAM regardless of file size.
- **In-memory shared state**: `Arc<Mutex<AppState>>` replaces disk-based IPC polling. No disk I/O during normal operation.
- **`flexi_logger`**: Logs written to both stdout (systemd) and `~/.cache/mimick/mimick.log` for persistent debugging.
- **Tray via `ksni`**: StatusNotifierItem tray using a `tokio::sync::watch` channel — no zombie processes, no D-Bus spawn.
- **Duplicate upload prevention**: `active_tasks` HashSet in the file monitor prevents multiple `wait_for_file_completion` tasks for the same file during long writes (e.g. screencasts).
- **App ID standardized**: Unified to `io.github.nicx17.mimick` across the binary, `.desktop`, `.metainfo.xml`, icons, and install scripts.
- **AppImage packaging**: `build_test_appimage.sh` compiles a release binary and assembles a standard AppDir in 5 steps.

### Changed

- Settings window uses hide-on-close (built once per process) — eliminates repeated GTK widget tree allocations.
- `ImmichApiClient` is a singleton (`OnceLock`) — single `reqwest` connection pool for the lifetime of the process.
- Autostart now uses `io.github.nicx17.mimick.desktop` symlink.
- All documentation (`ARCHITECTURE.md`, `DEVELOPMENT.md`, `TESTING.md`, `TROUBLESHOOTING.md`, `APPIMAGE_CREATION.md`) updated for Rust/Cargo.
- GitHub Actions release workflow updated for Rust toolchain.
- CodeQL analysis updated to use `languages: rust` with `build-mode: none`.

### Removed

- All Python source files (`main.py`, `settings_window.py`, `tray_icon.py`, etc.)
- `requirements.txt`, `pyproject.toml`, `setup.py`, `MANIFEST.in`

## [2.0.1] - 2026-03-08

### Changed

- Renamed repository and backend strings from `immich_sync_app` to `mimick`

## [2.0.0] - 2026-03-08

### Added

- **Complete Rebranding to Mimick**: Officially renamed the project from "Immich Sync" to "Mimick" to establish a unique identity and drop the generic moniker. All internal app IDs, metadata, documentations, and daemon variables have been fully synchronized.
- **GTK4 / libadwaita Migration**: Totally replaced the heavy PySide6 UI framework with a native, responsive GTK4 + libadwaita interface. The application now perfectly mimics the native look and feel of modern GNOME and KDE desktop environments.
- **Scalable Vector Icons**: Modernized app icon integration by deploying the high-resolution `mimick.svg` into system `hicolor/scalable/apps/` directories.

### Changed

- AppImage build scripts and installation loops have been completely restructured to support the new `mimick` nomenclature and GTK requirements.
- Standardized the GNOME window `StartupWMClass` bindings effectively preventing stray or duplicate launcher icons on Wayland/X11 desktops.

## [1.0.2] - 2026-03-07

### Fixed

- **AppImage Python 3.12 Bundle**: Overhauled AppImage scripts to download and bundle a standalone `python-build-standalone` payload, resolving missing C-Extension (`Pillow`) bugs on modern OS hosts (like Ubuntu 24).
- **GTK AppIndicator Native Support**: Added `PyGObject` to the packaged environment and successfully bridged host GUI drivers via `GI_TYPELIB_PATH` to ensure system tray icon features don't crash under isolated packaging.
- **Duplicate Album Creation Race Condition**: Implemented `threading.Lock()` on the `get_or_create_album` REST endpoint to ensure multiple simultaneous workers handling bulk image drops to new directories don't spawn multiple identical albums on the server if they bypass the cache at the same time.
- **Ubuntu 24 Tray Icon Crash**: Added graceful try/except block wrapping around the `TrayIcon/pystray` initialization loop. On modern Desktop Environments (Ubuntu 24 Wayland / Mutter) that deny AppIndicator injection, the application no longer permanently fails. Instead it safely disables the visual tray while dropping seamlessly into a headless background daemon. Launching from the GUI menu with the tray disabled intelligently loads the Settings Window.

## [1.0.1] - 2026-03-07

### Added

- **File Move/Rename Support**: `ImmichEventHandler` now captures `on_moved` watchdog events. Temporary file downloads (e.g. `video.mp4.tmp` from web browsers, rsync, Syncthing) that later rename internally to a valid media extension are now successfully captured and pushed to the upload queue.

### Fixed

- **Incomplete Video File Upload Bug (`wait_for_file_completion`)**: Prevented massive media files (like 30-minute GUI screencasts) from triggering early timeouts before they were fully written. Replaced absolute 10s wait logic with an adaptive 300-second _idle_ timeout loop; continuously growing items dynamically rest the counter keeping uploads safe regardless of copy duration.

## [1.0.0] - 2026-03-06

### Added

- **Animated UI Toggles**: Added custom beautiful `SlideSwitch` CSS animations to the Settings Window allowing users to visually toggle Internal (LAN) vs External (WAN) URL behaviors on and off.
- Config now persists `internal_url_enabled` and `external_url_enabled` booleans.
- Expanded testing coverage for `api_client` and `config` including advanced error-state simulation and file-system failure catching.

### Fixed

- **Captive Portal Bug Fix**: The API Ping routing logic now strictly requires a `{"res": "pong"}` JSON payload resolution to avoid falsely pinging local cafe Wi-Fi captive portals and breaking sync loops.
- **Failover Cache Reset Bug Fix**: Fixed an issue where a timeout connection to the Internal URL loop would not flush the active API endpoint causing the logic to effectively loop blindly instead of bouncing sequentially to the External URL.
- Fixed critical App UI freezing (App Not Responding) during testing connection pings syncing via a synchronous socket process - now visually wraps tests via Qt override wait cursors.
- **Queue Offline Resolution Fix**: Fixed a data-loss bug that permanently flushed queued failed uploads if the user closed the window. Implemented `~/.cache/mimick/retries.json` to seamlessly save pending cache limits, accompanied by an explicit background locking worker loop restoring files successfully.

## [0.2.0] - 2026-03-06

### Added

- AppImage distribution! A new fully packaged AppImage version of `mimick` is now available, bundling `PySide6` and all Python dependencies into a single, highly portable executable.
- Introduced `AI_CONTEXT.md` to help agentic tools understand the application's unique multi-threaded API architecture, system constraints, and X11/Wayland workarounds.

### Fixed

- Fixed critical Qt 6 Wayland connection error where the DBus portal rejected window launching (`Could not register app ID`). Application metadata is now strictly set before Qt engine initialization.
- Fixed a metadata warning regarding the `.desktop` suffix in Qt's `setDesktopFileName` handler.
- Fixed buggy AppRun bash script backslash escaping that was causing `Exec format error` exceptions inside generated `AppImage` distributions.
- Fixed a bug where native AppImages were trying to execute `main.py` outside of isolated module logic.

### Changed

- Promoted project status from Alpha to properly release `v0.2.0` (removed beta tags completely from code structure and internal About tags).
- Modified API `_ping` function tests from testing generic text formats to raw JSON validation checks.
- Added robust direct-file editing scripts to fully automate AppImage extraction, generation, and packaging (`build_test_appimage.sh`).
- Updated PySide6 dependencies and application system documentation (`ARCHITECTURE.md` and `DEVELOPMENT.md`).

## [0.1.0-alpha] - 2026-03-03

### Added

- Created `AppImage` deployment script and comprehensive guide for easy Linux distribution natively bundling `PySide6` and python libraries.
- Extended testing suite to cover `notifications`, `tray_icon`, and `state_manager` using fully mocked implementations.
- Implemented desktop entry integration and `install.sh` enhancements standardizing icons to `/usr/share/pixmaps`.
- Added new AppImage-specific helper scripts (`install-appimage.sh` and `uninstall-appimage.sh`).
- Added User Guide (`docs/USER_GUIDE.md`), Testing Guide (`docs/TESTING.md`), and Architecture Guide (`docs/ARCHITECTURE.md`) to assist end-users and developers.
- Added `CONTRIBUTING.md` and initial project scaffolding.
- Added modern structural badges and active Alpha-phase developmental warnings to the `README.md`.
- Properly credited application icon to Unsplash's Round Icons.

### Fixed

- Fixed issue on GNOME/X11 where the application icon would not render in the dock or settings window due to misaligned `.desktop` metadata (`StartupWMClass`).
- Revised the `install.sh` routine to ensure Python virtual environment integrity and `pip` availability before attempting dependency installation.

### Changed

- Transitioned project license from MIT to **GPL-3.0**.
- Refactored PySide6 window initializations to fallback to a reliable absolute image path as opposed to breaking natively on XDG theme engines lacking caching.
- Updated `pyproject.toml` and `setup.py` metadata for publishing (PyPI readiness), adding GPLv3 and Alpha classifiers.
