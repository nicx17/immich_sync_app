# Architecture

Mimick is a single-process Rust desktop daemon built on GTK4, Libadwaita, and Tokio.

## Core Pieces

- `src/main.rs`: application bootstrap, logger setup, single-instance behavior, and startup wiring
- `src/settings_window.rs`: GTK4/Libadwaita configuration UI
- `src/monitor.rs`: live filesystem watching and checksum preparation
- `src/startup_scan.rs`: launch-time catch-up scan for unsynced or retargeted files
- `src/queue_manager.rs`: upload workers, retry handling, and sync-index updates
- `src/api_client.rs`: Immich connectivity, uploads, album lookup, and duplicate-aware helpers
- `src/sync_index.rs`: local index of already-synced files
- `src/state_manager.rs`: persisted status snapshot for UI recovery
- `src/tray_icon.rs`: tray menu and signals back into the GTK main loop

## Runtime Flow

1. Mimick starts and initializes logging.
2. The primary instance loads config, keyring state, and cached state.
3. Live monitoring starts for the selected watch paths.
4. A startup scan walks those same watch folders and queues anything new, changed, or retargeted.
5. Queue workers upload or reassociate assets and update the local sync index.
6. The settings window reads shared in-memory state and can save config changes or request a restart.

## Persistence

- Config: `~/.config/mimick/config.json`
- Keyring: desktop secret storage
- Runtime cache: `~/.cache/mimick/`

Mimick avoids unnecessary disk writes during active syncing and writes retry/state files on graceful shutdown paths.
