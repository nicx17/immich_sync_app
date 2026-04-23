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

- **GNOME Platform 50 Runtime** (`org.gnome.Platform//50`)
- **GNOME SDK 50** (`org.gnome.Sdk//50`)
- **Freedesktop Rust Extension** (`org.freedesktop.Sdk.Extension.rust-stable//25.08`)

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
Console logs are colorized by level (error/warn/info/debug/trace).
File logging rotates automatically (approximately 2 MB per file, 5 files kept).

Increase verbosity with:

```bash
RUST_LOG=debug cargo run
```

## Settings UX & Apply Behavior

- Most UI changes in the Settings window are now applied live (auto-apply). This includes:
	- upload worker count
	- quiet hours start/end
	- folder add/remove
	- per-folder album target and folder rules

- Connectivity fields (API Key, Internal/External server URLs) are treated as save-only. Changes to these fields are applied only when the user clicks **Save** in the Connectivity section to avoid partially-applied network credentials during configuration edits.

See `/implementation.md` for the prioritized UX plan and acceptance criteria.

## Notifications

- Per-upload notifications were noisy when many workers were active. Mimick now aggregates worker outcomes into a single batch summary that states how many files were processed successfully and how many failed for the batch. Connectivity-related notifications (such as "Connection Lost") still fire independently to alert the user of network failures.


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
