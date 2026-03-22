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
5. folder rules are not excluding the file
6. Mimick is not paused because of manual control, metered networking, or battery-only behavior

## Diagnostics Bundle

Use `Export Diagnostics` from the Controls page to collect:

- `summary.txt`
- `config.json`
- `status.json`
- `retries.json`
- `synced_index.json`
- `mimick.log`

This is the easiest way to gather support details without exposing the API key.

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
