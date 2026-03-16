# Configuration and First Run

## First Launch

On first launch, open the settings window and configure:

1. `Internal URL` for LAN access to Immich.
2. `External URL` for WAN access to Immich.
3. An Immich API key.
4. One or more watch folders.

At least one URL must stay enabled.

## Watch Folders

Each watch folder can:

- sync into an existing album
- create a new album from a custom name
- use the folder name as the default album name

Flatpak builds only gain access to folders selected through the built-in picker.

## Run on Startup

Mimick can register itself to launch after login:

- Flatpak builds use the desktop background portal.
- Native builds create `~/.config/autostart/io.github.nicx17.mimick.desktop`.

## Save, Close, Quit

- `Save & Restart` writes the config and relaunches Mimick so watcher and connectivity changes take effect immediately.
- `Close` hides the settings window but keeps Mimick running.
- `Quit` exits the whole app.

The window close button behaves the same as `Close`.

## Config File

The main config file is:

`~/.config/mimick/config.json`

Important keys:

- `watch_paths`
- `internal_url`
- `external_url`
- `internal_url_enabled`
- `external_url_enabled`
- `run_on_startup`

The API key is stored in the system keyring, not in `config.json`.
