# Troubleshooting

## Tray Icon Missing

Some desktops restrict or hide legacy tray icons.

- GNOME often needs the AppIndicator/KStatusNotifier extension.
- If tray support is unavailable, launching Mimick again should still open the running instance's settings window.

## Notifications Look Wrong

Some lightweight notification daemons do not support replacement or progress hints well. In that case you may see multiple notifications instead of a single updating progress item.

## Files Are Not Syncing

Check:

1. the watch folder was selected through the app
2. the file extension is supported
3. the file finished writing to disk
4. the API key and server URLs are valid

## Useful Logs

Persistent log:

```bash
tail -f ~/.cache/mimick/mimick.log
```

Terminal run:

```bash
cargo run
```

Both terminal and file logs include timestamps, levels, and source modules.

## Cache Files

Important runtime files:

- `~/.cache/mimick/mimick.log`
- `~/.cache/mimick/retries.json`
- `~/.cache/mimick/synced_index.json`
- `~/.cache/mimick/status.json`
