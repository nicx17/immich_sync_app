# Testing Guide: Immich Auto-Sync

This document outlines how to execute, interpret, and expand the automated testing suite for the Immich Auto-Sync application.

## 1. The Testing Framework
The application uses **`pytest`** as the primary testing runner. 
Mocks (simulating system behavior without actually modifying files or hitting the real network) are handled by the standard library's `unittest.mock` and the `requests_mock` plugin.

### Prerequisites
Ensure your virtual environment is active and the testing dependencies are installed:
```bash
source .venv/bin/activate
pip install pytest pytest-cov requests-mock
```

---

## 2. Running Tests

To run the entire test suite simply execute:
```bash
pytest tests/
```

### Checking Code Coverage
To see how much of your actual application code is being tested, use the `pytest-cov` plugin:
```bash
# Basic coverage overview
pytest tests/ --cov=src

# Detailed view (shows exactly which line numbers are missing tests)
pytest tests/ --cov=src --cov-report=term-missing
```

---

## 3. Test File Structure

The tests are located in the `tests/` directory and mirror the structure of `src/`.

| Test File | Description | Current Coverage Target |
| :--- | :--- | :--- |
| `test_api_client.py` | Tests Immich server connectivity (ping), WAN/LAN fallback routing, and successful/failed upload responses. | `src/api_client.py` |
| `test_config.py` | Tests JSON parsing, default file creation, and secure keyring interactions. | `src/config.py` |
| `test_monitor.py` | Tests the `watchdog` event handler. Ensures valid extensions trigger syncs, while directories and invalid extensions are ignored. | `src/monitor.py` |
| `test_queue_manager.py` | Tests internal SQLite queue adding, task generation, and multi-threaded worker processing behavior. | `src/queue_manager.py` |
| `test_utils.py` | Tests isolated helper functions like SHA-1 checksum generation. | `src/utils.py` |
| `test_notifications.py` | Mocks `subprocess.Popen` to ensure desktop `notify-send` commands are formatted correctly. | `src/notifications.py` |
| `test_state_manager.py` | Tests atomic JSON file writes used to synchronize progress between the daemon and UI. | `src/state_manager.py` |
| `test_tray_icon.py` | Mocks `pystray` system tray elements to verify initialization and application quit logic. | `src/tray_icon.py` |

---

## 4. Current Coverage Gaps (To-Do)

While backend logic (Queue, Config, Notifier) is highly tested (~80-100%), the following areas currently have **0% coverage** by design and require manual testing:

1.  **`src/settings_window.py` / `src/settings_main.py`**: PySide6 GUI views. Automated testing for these requires complex graphical emulators (like `xvfb` and `pytest-qt`).
2.  **`src/main.py`**: The main entry point daemon logic. It only handles command-line arguments and starting other systems, making it difficult to test without starting actual infinite loops.

## 5. Writing New Tests

When adding a new feature to `src/`, always create a corresponding `test_*.py` file or function.

**Best Practices:**
*   **Never hit the real network:** Use `requests_mock` to pretend you are the Immich server.
*   **Never modify the real disk:** Use `unittest.mock.patch('builtins.open')` or pytest's `tmp_path` fixture to handle file I/O safely.
*   **Keep them fast:** Tests should not contain real `time.sleep()` delays. Mock the `time` module if needed.
