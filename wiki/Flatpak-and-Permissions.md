# Flatpak and Permissions

Mimick is designed to work with selected-folder access rather than full home-directory access.

## Folder Access

Flatpak builds do not need `--filesystem=home`.

Instead, folders are granted one at a time through the file chooser portal. Inside the sandbox those folders may appear as document-portal paths such as:

`/run/user/1000/doc/...`

The Mimick UI and logs convert those into friendlier folder names where possible.

## Credential Storage

Mimick uses the [oo7](https://github.com/linux-credentials/oo7) crate for secure API key storage. The backend is selected automatically:

- **Inside Flatpak**: Credentials are stored in an encrypted file within the sandbox (`~/.var/app/io.github.nicx17.mimick/data/keyrings/`). The encryption key is retrieved from the `org.freedesktop.portal.Secret` portal. This avoids exposing secrets to other sandboxed applications.
- **Outside Flatpak (native)**: Credentials are stored in the desktop's Secret Service (GNOME Keyring, KWallet) via the `org.freedesktop.secrets` D-Bus interface.

### D-Bus Permissions

The Flatpak manifest grants the following D-Bus talk permissions:

| Permission | Purpose |
|---|---|
| `org.kde.StatusNotifierWatcher` | System tray icon integration |

## Tray Integration

Mimick uses StatusNotifier support for its tray icon.

The Flatpak manifest keeps this narrowed to the minimum needed integration and no longer requests broad `org.kde.*` bus-name ownership.

## Background Launch

The `Run on Startup` option uses:

- the desktop background portal in Flatpak
- a regular autostart desktop entry outside Flatpak

## Local and Deployed Builds

The local Flatpak manifest follows the same selected-folder model as the deployed build so behavior stays consistent during testing.
