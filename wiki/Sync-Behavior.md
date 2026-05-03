# Sync Behavior

Mimick uses two sync paths: a startup catch-up scan and live filesystem monitoring.

## Startup Catch-Up

When Mimick launches, it scans the configured watch folders for supported media files.

It keeps a local sync index in `~/.cache/mimick/synced_index.json` so it can:

- skip unchanged files that are already known to be synced
- detect files whose content changed and requeue them
- detect when a watch folder's target album changed

## Live Monitoring

While the app is running, Mimick watches the selected folders for new or modified files.

Before upload, it:

1. waits for file writes to settle
2. computes a SHA-1 checksum
3. uploads the file to Immich
4. adds the asset to the target album

Live monitoring and startup scans both use the most specific matching watch-folder configuration, so nested folders inherit the correct album and rule set instead of falling back to a less-specific parent path.

## Album Reassociation

If you retarget a watch folder to a different Immich album, Mimick can reassociate unchanged files on a later startup without forcing a full reupload.

If the old album was deleted or the stored album ID is stale, Mimick refreshes album resolution and retries using the current configured album name.

## Retry and Duplicate Handling

- Failed uploads are stored in `~/.cache/mimick/retries.json` and retried on the next run.
- Duplicate detection uses SHA-1 checksums and Immich's upload-check behavior.
- Existing assets can be looked up and reused instead of uploading the file bytes again.
- The Status page can pause/resume syncing, trigger `Sync Now`, and expose queue recovery actions while Mimick is already running.

## Album Synchronization (Bidirectional)

With the introduction of the built-in Library View, an on-demand bidirectional sync between local folders and remote albums is supported. When a local folder is linked to a remote album:

1. **Difference Calculation:** Mimick compares existing local files to assets in the remote album using their SHA-1 checksums.
2. **Uploading:** Files inside the local folder that are missing remotely are queued for upload to the target album.
3. **Downloading:** Assets in the target album missing locally are downloaded to the linked watch folder. 
4. **Collision Handling:** If a file with an identical name already exists but hasn't fully matched via SHA-1 or needs renaming, Mimick automatically appends a numeric suffix (e.g. `file (1).jpg`) to prevent clobbering existing local data.
