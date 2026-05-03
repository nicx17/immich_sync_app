# Performance Tuning

Default settings work for most setups. The options below are useful when you have a large library, limited hardware, or want to control how aggressively Mimick uses bandwidth and memory.

---

## Upload Workers

**Setting:** Settings → Behavior → Upload Workers (1–10, default 3)

Each worker is an independent async upload task. Workers share the same HTTP connection pool, so adding more workers increases concurrent uploads rather than opening separate connections.

| Scenario | Suggested value |
| :--- | :--- |
| Slow or metered connection | 1–2 |
| Home LAN, typical library | 3 (default) |
| Fast LAN or local server, large backlog | 5–8 |
| Saturating uploads cause other issues | Reduce back toward default |

Workers apply to the background sync engine only. Album sync (triggered from the library view) runs as a separate task and is not affected by this setting.

Changing the worker count takes effect immediately after saving — no restart needed.

---

## Startup Catch-Up Mode

**Setting:** Settings → Behavior → Startup Catch-Up Mode

Controls how much work Mimick does each time it launches.

| Mode | What it does | When to use it |
| :--- | :--- | :--- |
| **Full** | Hashes every file in every watch folder | Initial setup; after adding many new folders |
| **Recent Only** | Hashes files modified in approximately the last 7 days | Ongoing use with occasional gaps |
| **New Files Only** | Only processes files not already in the sync index | Daily driver once the library is fully synced |

Switch from Full to New Files Only once your initial sync is complete. Startup time on a large library (tens of thousands of files) drops significantly.

---

## Thumbnail Memory Cache

**Setting:** Settings → Behavior → Thumbnail Memory Cache (MB, default 80, range 16–1024)

The library view decodes thumbnails into RAM for fast rendering. This cap controls how much memory decoded thumbnails can occupy before older entries are evicted.

- Increasing the cap speeds up scrolling back through previously visited pages.
- Decreasing it frees RAM at the cost of re-fetching thumbnails when scrolling back.
- On systems with limited RAM, leave at default or reduce to 32–48 MB.
- On systems with ≥16 GB RAM and a large library, 256–512 MB can noticeably reduce re-fetches.

Changing the cache size takes effect after restarting Mimick.

---

## Full-Resolution Preview

**Setting:** Settings → Behavior → Full-Resolution Preview (switch, default off)

When off, the lightbox loads a ~1440px server-generated preview image. When on, it loads the original file.

- Full-resolution mode uses significantly more bandwidth and takes longer to load on slow connections.
- Use it only if you need to inspect originals or are on a fast LAN connection to your Immich server.

---

## Large Libraries: Recommended Configuration

For libraries with more than ~50,000 files:

1. Set **Startup Catch-Up Mode** to **New Files Only** after the initial sync.
2. Keep **Upload Workers** at 3–5 unless uploads are a bottleneck.
3. Set **Thumbnail Memory Cache** to 256 MB or higher if RAM allows.
4. Leave **Full-Resolution Preview** off unless needed.
