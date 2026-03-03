# Architecture Overview

This document describes the high-level architecture of `immich-sync`, a Linux desktop daemon for syncing media to Immich.

## System Components

The application is designed as a multi-threaded daemon with a decoupled UI process.

```mermaid
graph TD
    User[User] -->|Interacts| Tray[System Tray Icon]
    Tray -->|Launches| Settings[Settings Window (PySide6)]
    
    subgraph Daemon [Background Service]
        Main[main.py] --> Monitor[File Monitor (Watchdog)]
        Main --> Tray
        
        Monitor -->|File Events| Queue[Queue Manager]
        Queue -->|Distributes| Workers[Worker Threads (x10)]
        
        Workers -->|Upload| API[Immich API Client]
        Workers -->|Update| State[State Manager]
        Workers -->|Notify| Notify[Notification Manager]
    end
    
    subgraph Storage
        Config[config.json]
        Keyring[System Keyring]
        StateFile[status.json]
    end
    
    Settings -->|Read/Write| Config
    Settings -->|Read only| Keyring
    Settings -->|Read only| StateFile
    
    State -->|Write| StateFile
    Queue -->|Read| Keyring
```

### 1. Core Daemon (`src/main.py`)
The entry point of the application. It performs environment setup (forcing X11 backend for GDK to avoid Wayland issues with legacy tray icons) and initializes the `Monitor` and `TrayIcon`.

### 2. File Monitor (`src/monitor.py`)
Uses the `watchdog` library to listen for filesystem events (`IN_CREATE`, `IN_MOVED_TO`).
- **Event Handling**: Filters events to allow only specific media extensions (`.jpg`, `.png`, `.mp4`, etc.).
- **Debouncing**: Implements a `wait_for_file_completion` check to ensure files are fully written to disk before processing.
- **Checksum**: Calculates SHA-1 checksums immediately to support Immich's deduplication logic.

### 3. Queue Manager (`src/queue_manager.py`)
A thread-safe orchestrator for upload tasks.
- **Upload Queue**: A FIFO queue receiving file tasks from the Monitor.
- **Retry Queue**: Captures failed uploads for later retry (exponential backoff not yet fully implemented).
- **Worker Pool**: Spawns 10 daemon threads to process uploads in parallel.
- **Progress Tracking**: updates the `StateManager` with current progress.

### 4. API Client (`src/api_client.py`)
Encapsulates communication with the Immich Server.
- **Dual-URL Support**: Checks logical connectivity to Internal (LAN) URL first, falling back to External (WAN) URL.
- **Asset Upload**: Handles `multipart/form-data` uploads.
- **Album Management**: Checks if a target album exists (based on folder name) and creates it if missing.

### 5. Settings UI (`src/settings_window.py`)
A standalone `PySide6` (Qt) application that runs firmly in a separate process from the daemon.
- **Reasoning**: Mixing GTK (used by `pystray`/`AppIndicator`) and Qt event loops in the same process causes instability and crashes.
- **Communication**: Reads status from `~/.cache/immich-sync/status.json` to display the progress bar.

### 6. Notifications & State
- **Notifications**: Uses `notify-send` via `subprocess` to display native desktop notifications with progress bars.
- **State Manager**: Serializes the daemon's current activity (queue size, current file) to a JSON file for the UI to consume.
