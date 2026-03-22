# Testing

Run the full unit test suite with:

```bash
cargo test
```

## What Is Covered

Current tests focus on:

- config parsing and defaults
- watch-path matching for nested folder configs
- checksum generation
- queue duplicate prevention and retry controls
- retry persistence
- sync-index decisions
- watch-path display helpers
- restart request handling
- selected autostart helpers

## Known Gaps

Areas that still depend more heavily on manual testing:

- GTK settings UI behavior
- tray integration across different desktops
- end-to-end network behavior against a live Immich server
- full Flatpak desktop integration in real sessions

## Testing Guidance

- Prefer fast inline unit tests inside the source file under `#[cfg(test)]`.
- Use `tempfile` for filesystem tests.
- Avoid hitting the real network in automated tests.
