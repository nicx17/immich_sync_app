# Contributing to Immich Auto-Sync

First off, thank you for considering contributing to Immich Auto-Sync for Linux! It's people like you that make open-source software great.

## Code of Conduct
By participating in this project, you agree to abide by common open-source standards of respect and collaboration. Be welcoming, be kind, and keep feedback constructive.

## How Can I Contribute?

### 1. Reporting Bugs
If you find a bug, please open an Issue on GitHub. Include:
* Your Operating System and Desktop Environment (e.g., Ubuntu 22.04, GNOME Wayland).
* Your python version.
* A clear description of the bug and steps to reproduce.
* Logs if possible (can be viewed by running the program via terminal).

### 2. Suggesting Enhancements
Have an idea for a new feature? 
* First, check the `roadmap.md` file to see if it is already planned.
* Second, check existing issues to see if someone else has suggested it.
* If not, open a feature request Issue detailing how the feature should work and why it would be beneficial.

### 3. Pull Requests
We gladly accept Pull Requests (PRs).

**Workflow:**
1. Fork the repo and create your branch from `main`.
2. Ensure you have the dev dependencies installed (`pip install -r requirements.txt`).
3. Make your changes in your branch.
4. If you've added code that should be tested, **add tests** corresponding to your logic in the `tests/` directory.
5. Run the test suite (`pytest tests/`) to ensure nothing is broken.
6. Submit your PR!

### 4. Code Style
* The project generally adheres to PEP 8 standards.
* Logic testing is heavily prioritized over GUI testing. Ensure any new modules have corresponding mocked unit tests.
* Avoid adding complex C-bindings or system dependencies where possible to maintain the portability of the AppImage.

## Development Environment Setup
Please refer to `docs/DEVELOPMENT.md` for a comprehensive guide on setting up your local environment for editing the UI or backend services.
