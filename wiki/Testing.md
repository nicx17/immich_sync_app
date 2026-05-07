# Testing Guide

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
| `src/api_client.rs` | `mod tests` | Tests upload-check response parsing, album resolution helpers, and endpoint URL selection. |
| `src/autostart.rs` | `mod tests` | Tests desktop-exec escaping and Flatpak host config path mapping. |
| `src/config.rs` | `mod tests` | Tests JSON serde behavior, folder-rule matching, hidden-path filtering, extension normalization, and best-match watch-path selection. |
| `src/diagnostics.rs` | `mod tests` | Tests support-summary generation and diagnostics bundle export contents. |
| `src/library/local_source.rs` | `mod tests` | Tests synthetic ID generation, case-insensitive filename filtering, and query matching. |
| `src/library/state.rs` | `mod tests` | Tests pagination reset rules, search clearing behaviors, and duplicate request suppression. |
| `src/library/thumbnail_cache.rs` | `mod tests` | Tests ThumbHash hit/miss logic, memory eviction budgets, and disk removal. |
| `src/main.rs` | `mod tests` | Tests live-queue watch-path selection so nested folders use the most specific configured target. |
| `src/monitor.rs` | `mod tests` | Tests monitor-side filtering such as temporary-file detection, media extension matching, and SHA-1 checksum computation. |
| `src/queue_manager.rs` | `mod tests` | Tests duplicate queue prevention, retry persistence, failed-queue clearing, and manual retry requeueing. |
| `src/runtime_env.rs` | `mod tests` | Tests metered-network parsing and battery-power decision logic without depending on the host system. |
| `src/settings_window.rs` | `mod tests` | Tests pure-logic helpers used by the settings UI such as config field validation. |
| `src/startup_scan.rs` | `mod tests` | Tests startup-scan file filtering and sync-index interaction for catch-up logic. |
| `src/state_manager.rs` | `mod tests` | Tests queue-event updates, event-history truncation rules, and health dashboard field persistence. |
| `src/sync_index.rs` | `mod tests` | Tests sync record creation, album-change detection, and index persistence round-trips. |
| `src/watch_path_display.rs` | `mod tests` | Tests document-portal path detection and user-friendly folder label generation. |

---

## 4. Current Coverage Gaps

While core data structures and support logic are tested, the following areas still have **limited coverage** and rely heavily on manual UI or integration testing during development:

1. **`src/settings_window.rs`**: GTK4/libadwaita UI interactions such as dialogs, queue inspector rendering, and per-folder rules editing. Some pure-logic helpers are covered, but widget behavior is not.
2. **`src/library/*` UI Views**: The complex GTK4 interactions for the Photos grid, Context Menus, Album Sync dialogs, and Explore views are primarily verified via manual testing rather than automated tests.
3. **`src/api_client.rs`**: Basic response parsing is tested, but network endpoints still need deeper mocked or sandboxed Immich coverage.
4. **`src/main.rs` / `src/tray_icon.rs`**: Full daemon lifecycle, tray signaling, and application-instance behavior remain mostly manual/integration tested.
5. **Async worker integration**: Queue manager helpers are covered more directly now, but end-to-end worker behavior against a mocked API is still a worthwhile next step.

## 5. Writing New Tests

When adding a new feature, always consider creating a corresponding inline `#[test]` function.

**Best Practices:**
*   **Never hit the real network:** Use a mock HTTP responder if testing API consumers.
*   **Never modify the real disk:** Use the `tempfile` crate (already in `[dev-dependencies]`) to create temporary, auto-cleaning directories for file I/O tests.
*   **Keep them fast:** Do not inject artificial `tokio::time::sleep()` delays unless absolutely necessary for channel sync tests.
*   **Prefer pure helpers for environment-sensitive logic:** Parse command output or power-supply state via helper functions so the behavior can be tested deterministically.
