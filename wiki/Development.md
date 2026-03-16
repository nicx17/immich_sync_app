# Development

## Prerequisites

- Rust toolchain via `rustup`
- GTK4 development packages
- Libadwaita development packages
- Libsecret development packages

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
