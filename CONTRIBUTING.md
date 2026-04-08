# Contributing to Mimick

Thank you for considering contributing to Mimick for Linux!

## Code of Conduct

Be welcoming, kind, and keep feedback constructive.

## How Can I Contribute?

### 1. Reporting Bugs

Open an Issue on GitHub. Include:
* Your OS and desktop environment (e.g., Ubuntu 24.04, GNOME Wayland 50).
* Rust and GTK4/Libadwaita versions (`rustc --version`, `pkg-config --modversion gtk4`).
* A clear description and steps to reproduce.
* Logs from a terminal run (`RUST_LOG=debug mimick 2>&1`).

### 2. Suggesting Enhancements

* Check `roadmap.md` to see if it is already planned.
* Check open issues for duplicates.
* If not, open a feature request Issue.

### 3. Pull Requests

**Workflow:**

1. Fork the repo and create your branch from `main`.
2. Install the system prerequisites (see README).
3. Build with `cargo build`.
4. If you add logic, add unit tests in the same file under `#[cfg(test)]`.
5. Run tests: `cargo test`.
6. Ensure no warnings: `cargo clippy`.
7. Submit your PR.

### 4. Code Style

* Follow standard Rust formatting: `cargo fmt` before committing.
* Use `cargo clippy` and address all lints — no deprecated APIs.
* Unit tests live in the same source file as the code under `#[cfg(test)] mod tests { ... }`.
* GUI code in `settings_window.rs` is intentionally not unit tested — focus on the pure-logic modules.

## Development Environment Setup

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install GTK system dependencies (Ubuntu/Debian)
sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-dev pkg-config build-essential libsecret-1-dev

# Build and run
cargo run -- --settings

# Run tests
cargo test
```
