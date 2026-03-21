# Testing Guide: Mimick

This document outlines how to execute and expand the automated testing suite for the Mimick application.

## 1. The Testing Framework
The application uses the standard **`cargo test`** runner built into Rust.

### Prerequisites
Ensure your Rust toolchain is up to date:
```bash
rustup update stable
```

---

## 2. Running Tests

To run the entire test suite simply execute:
```bash
cargo test
```

To mirror the main CI quality gate locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

### Checking Specific Modules
You can target specific modules or functions:
```bash
# Run tests only in monitor.rs
cargo test monitor::

# Run tests with output printed to terminal (normally hidden on success)
cargo test -- --nocapture
```

---

## 3. Test File Structure

Tests in Rust are written inline within identical files to the logic they test, placed inside `#[cfg(test)]` modules at the bottom of the files.

| Source File | Test Location | Description |
| :--- | :--- | :--- |
| `src/config.rs` | `mod tests` | Tests JSON serde behavior plus folder-rule matching, hidden-path filtering, and extension normalization. |
| `src/monitor.rs` | `mod tests` | Tests monitor-side filtering such as temporary-file detection before queueing. |
| `src/runtime_env.rs` | `mod tests` | Tests metered-network parsing and battery-power decision logic without depending on the host system. |
| `src/diagnostics.rs` | `mod tests` | Tests support-summary generation and diagnostics bundle export contents. |
| `src/state_manager.rs` | `mod tests` | Tests queue-event updates and event-history truncation rules. |

---

## 4. Current Coverage Gaps

While core data structures and support logic are tested, the following areas still have **limited coverage** and rely heavily on manual UI or integration testing during development:

1. **`src/settings_window.rs`**: GTK4/libadwaita UI interactions such as dialogs, queue inspector rendering, and per-folder rules editing.
2. **`src/api_client.rs`**: Network endpoints still need deeper mocked or sandboxed Immich coverage.
3. **`src/main.rs` / `src/tray_icon.rs`**: Full daemon lifecycle, tray signaling, and application-instance behavior remain mostly manual/integration tested.
4. **`src/queue_manager.rs`**: Core behavior is exercised indirectly, but more direct async worker tests would still be valuable.

## 5. Writing New Tests

When adding a new feature, always consider creating a corresponding inline `#[test]` function.

**Best Practices:**
*   **Never hit the real network:** Use a mock HTTP responder if testing API consumers.
*   **Never modify the real disk:** Use the `tempfile` crate (already in `[dev-dependencies]`) to create temporary, auto-cleaning directories for file I/O tests.
*   **Keep them fast:** Do not inject artificial `tokio::time::sleep()` delays unless absolutely necessary for channel sync tests.
*   **Prefer pure helpers for environment-sensitive logic:** Parse command output or power-supply state via helper functions so the behavior can be tested deterministically.
