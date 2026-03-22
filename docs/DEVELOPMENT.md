# Development Guide

This guide is for developers who want to contribute to `mimick`.

## Setting Up the Environment

### Prerequisites

- Rust toolchain (`cargo` + `rustc`) via [rustup](https://rustup.rs/)
- GTK4 development files
- Libadwaita development files
- libsecret development files (for system keyring access)

### Installation

1. **Clone the Repository:**

    ```bash
    git clone https://github.com/nicx17/mimick.git
    cd mimick
    ```

2. **Install Dependencies (Ubuntu/Debian):**

    ```bash
    sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-dev pkg-config build-essential libsecret-1-dev
    ```

3. **Install Dependencies (Fedora):**

    ```bash
    sudo dnf install gtk4-devel libadwaita-devel libsecret-devel pkg-config
    ```

4. **Install Dependencies (Arch Linux):**

    ```bash
    sudo pacman -S gtk4 libadwaita libsecret pkgconf base-devel
    ```

5. **Build and Run:**

    ```bash
    cargo check             # Check if code compiles without building
    cargo run               # Run in background daemon mode
    cargo run -- --settings # Run and immediately open the settings window
    ```

6. **Lint and Test Before Opening a PR:**

    ```bash
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test
    ```

## Logging and Debugging

The application uses `flexi_logger`. 
- By default, `cargo run` prints `INFO` level logs to the terminal.
- Logs are simultaneously written to disk at `~/.cache/mimick/mimick.log`.
- Both terminal and file logs use timestamped detailed formatting.
- To increase verbosity, set the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug cargo run
```

## Running Tests

The project uses Rust's built-in testing framework for configuration parsing, folder rules, queue-state bookkeeping, diagnostics export, monitor filtering, runtime environment parsing, and other non-UI behavior.

```bash
cargo test
```

## UI Structure and Main Loop

Unlike traditional Python/PySide loops, `mimick` is built on GTK4 and multi-threaded `tokio`.

1. `main.rs`: Initialises the GTK `adw::Application` and spins up the background `tokio` runtime for the file monitor and network queue.
2. `settings_window.rs`: Uses declarative GTK Builder pattern to construct the UI. The UI reads status via a shared `Arc<Mutex<AppState>>` memory lock rather than disk polling.
3. GTK restricts all UI modifications to the main thread. To update the UI from async workers, use generic channels or `glib::timeout_add_local`.

Recent operational features are spread across a few focused modules:

- `queue_manager.rs`: upload workers, retry controls, pause/resume state, queue event recording
- `state_manager.rs`: persisted app state plus recent queue-event history
- `runtime_env.rs`: best-effort metered-network and battery-power detection
- `diagnostics.rs`: support bundle export that redacts secrets, raw logs, URLs, and full local paths
- `settings_window.rs`: queue inspector, diagnostics export, per-folder rule editing, and manual sync controls

## Packaging

To test the final executable bundle via Flatpak:

```bash
flatpak-builder --user --install --force-clean build-dir io.github.nicx17.mimick.local.yml
```

Once installed, you can run it via your application menu or `flatpak run io.github.nicx17.mimick`.
Note that modifying `Cargo.toml` or `Cargo.lock` requires you to re-run `uv run flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json` so the Flatpak offline vendor set stays in sync.

GitHub Actions currently mirrors the same native quality gate with:

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test --locked`

The published Flatpak repository is built in a containerized Flatpak workflow rather than by installing Flatpak tooling directly on the host runner.

Repository automation details such as Dependabot, CODEOWNERS, Release Drafter, docs link checks, and the Flatpak vendor guard are documented in [REPOSITORY_AUTOMATION.md](REPOSITORY_AUTOMATION.md).

## Contributing Workflow

1. **Fork** the repository.
2. **Clone** your fork.
3. Create a **feature branch**: `git checkout -b feature/my-new-feature`.
4. Run formatting, clippy, and tests locally.
5. Commit your changes.
6. Submit a **Pull Request**.
