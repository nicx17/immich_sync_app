# Development Guide

This guide is for developers who want to contribute to `immich-sync`.

## Setting Up the Environment

### Prerequisites
- Python 3.10+
- `venv` (recommended)
- `pip`
- `pytest`
- `glib-2.0` / `gobject-introspection` development files (for Pystray/PySide6)

### Installation

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/your-repo/immich-sync.git
    cd immich-sync
    ```

2.  **Create Virtual Environment:**
    ```bash
    python -m venv .venv
    source .venv/bin/activate
    ```

3.  **Install Dependencies:**
    ```bash
    pip install -r requirements.txt
    ```

## Running Tests

The project uses `pytest` for unit testing.
```bash
pytest tests/
```

### Coverage
To check test coverage:
```bash
pip install pytest-cov
pytest --cov=src tests/
```

## Packaging

### Arch Linux (PKGBUILD)
To build an Arch package:
1.  Navigate to `setup/`.
2.  Update the `pkgver` and `sha256sums` (using `sha256sum setup.py`).
3.  Run `makepkg -si`.

### Python Package (pip)
This project includes a `setup.py`. You can install it locally in editable mode:
```bash
pip install -e .
```

## Contributing Workflow

1.  **Fork** the repository.
2.  **Clone** your fork.
3.  Create a **feature branch**: `git checkout -b feature/my-new-feature`.
4.  Commit your changes: `git commit -am 'Add some feature'`.
5.  Push to the branch: `git push origin feature/my-new-feature`.
6.  Submit a **Pull Request**.

### Coding Standards
- Follow PEP 8 guidelines.
- Use meaningful variable names.
- Document complex functions with docstrings.
- Add new tests for new functionality.
