# Development

## Prerequisites

- Rust toolchain via `rustup`
- GTK4 development packages
- Libadwaita development packages

### Ubuntu / Debian

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-dev pkg-config build-essential
```

### Fedora

```bash
sudo dnf install gtk4-devel libadwaita-devel pkg-config
```

### Arch Linux

```bash
sudo pacman -S gtk4 libadwaita pkgconf base-devel
```

### Flatpak Packaging Prerequisites

If you plan to build or test the Flatpak bundle locally using `flatpak-builder`, you must install the following from [Flathub](https://flathub.org/setup):

- **GNOME Platform 49 Runtime** (`org.gnome.Platform//49`)
- **GNOME SDK 49** (`org.gnome.Sdk//49`)
- **Freedesktop Rust Extension** (`org.freedesktop.Sdk.Extension.rust-stable//24.08`)

## Build and Run

```bash
cargo check
cargo run
cargo run -- --settings
```

## Logging

Mimick uses `flexi_logger` and writes logs to both:

- stdout
- `~/.cache/mimick/mimick.log`

Detailed timestamps are enabled for both outputs.

Increase verbosity with:

```bash
RUST_LOG=debug cargo run
```

## Packaging

For local Flatpak work:

```bash
flatpak-builder --user --install --force-clean build-dir io.github.nicx17.mimick.local.yml
```

Run the staged Flatpak without installing:

```bash
flatpak-builder --run build-dir io.github.nicx17.mimick.local.yml mimick --settings
```

### Regenerating Cargo Sources for Flatpak

After updating `Cargo.toml` or `Cargo.lock`, regenerate the Flatpak cargo sources manifest:

```bash
python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
```
