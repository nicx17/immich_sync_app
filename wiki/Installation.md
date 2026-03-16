# Installation

The recommended install method is the official Flatpak repository.

## Flatpak

```bash
flatpak remote-add --user --if-not-exists mimick-repo https://nicx17.github.io/mimick/mimick.flatpakrepo
flatpak install --user mimick-repo io.github.nicx17.mimick
```

Run the app with:

```bash
flatpak run io.github.nicx17.mimick
```

Open the settings window directly with:

```bash
flatpak run io.github.nicx17.mimick --settings
```

## Local Development Build

For a native development run:

```bash
cargo run
```

Open settings immediately:

```bash
cargo run -- --settings
```

For a local Flatpak build that uses the current checkout instead of the GitHub source tag:

```bash
flatpak-builder --user --install --force-clean build-dir io.github.nicx17.mimick.local.yml
```

## What Gets Installed

- Application ID: `io.github.nicx17.mimick`
- Binary: `mimick`
- Config file: `~/.config/mimick/config.json`
- Cache directory: `~/.cache/mimick/`
