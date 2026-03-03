#!/bin/bash
# install.sh - Automated Installation Script

set -e

APP_NAME="immich-sync"
ICON_NAME="immich-sync"
DESKTOP_FILE="immich-sync.desktop"
VENV_DIR=".venv"

# Directories
USER_APPS="$HOME/.local/share/applications"
USER_ICONS_SCALABLE="$HOME/.local/share/icons/hicolor/scalable/apps"
USER_ICONS_PNG="$HOME/.local/share/icons/hicolor/128x128/apps"
AUTOSTART_DIR="$HOME/.config/autostart"

# Ensure directories exist
mkdir -p "$USER_APPS"
mkdir -p "$USER_ICONS_SCALABLE"
mkdir -p "$USER_ICONS_PNG"
mkdir -p "$AUTOSTART_DIR"

echo "=== Immich Sync Installer ==="

# 1. Environment Setup
echo "[1/4] Setting up Python environment..."
CWD=$(pwd)

# Check if python3 is available
if ! command -v python3 &> /dev/null; then
    echo "Error: python3 could not be found."
    exit 1
fi

# Check for python3-venv (Common issue on Debian/Ubuntu)
if ! python3 -c "import venv" &> /dev/null; then
    echo "Error: The 'venv' module is missing."
    echo "On Debian/Ubuntu systems, please install it via:"
    echo "  sudo apt install python3-venv"
    exit 1
fi

# Create venv if it doesn't exist
if [ ! -d "$VENV_DIR" ]; then
    echo "Creating virtual environment in $VENV_DIR..."
    python3 -m venv "$VENV_DIR"
else
    echo "Virtual environment found in $VENV_DIR."
fi

# Activate venv for installation context
source "$VENV_DIR/bin/activate"

# Check if pip is available in venv
if ! command -v pip &> /dev/null; then
    echo "Error: pip is missing from the virtual environment."
    echo "Attempting to bootstrap pip..."
    python3 -m ensurepip --default-pip
# Verify installation of key UI libraries that often fail due to missing system deps
if ! python3 -c "import pystray" &> /dev/null; then
    echo "Warning: 'pystray' failed to import. You might be missing system libraries."
    echo "On Debian/Ubuntu: sudo apt install libgirepository1.0-dev libcairo2-dev"
    echo "On Fedora: sudo dnf install gobject-introspection-devel cairo-devel"
fi

fi

# Install dependencies
echo "Installing dependencies from requirements.txt..."
pip install --upgrade pip > /dev/null
pip install -r requirements.txt

PYTHON_EXEC="$CWD/$VENV_DIR/bin/python3"
SCRIPT_PATH="$CWD/src/main.py"

# 2. Install Icons
echo "[2/4] Installing icons..."

# Install SVG (Scalable) - Standard XDG location
cp setup/icons/immich-sync.svg "$USER_ICONS_SCALABLE/$ICON_NAME.svg"

# Install PNG (128x128) - Standard XDG location
cp src/assets/icon.png "$USER_ICONS_PNG/$ICON_NAME.png"

# Optional: Install to /usr/share/pixmaps for legacy support (requires sudo)
# We prompt for this because it modifies system directories
if [ -w "/usr/share/pixmaps" ]; then
    echo "Copying icon to /usr/share/pixmaps (System-wide)..."
    cp src/assets/icon.png "/usr/share/pixmaps/$ICON_NAME.png"
else
    echo "Note: To ensure maximum compatibility, we can also install the icon to /usr/share/pixmaps."
    read -p "Install icon to /usr/share/pixmaps (requires sudo)? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        sudo cp src/assets/icon.png "/usr/share/pixmaps/$ICON_NAME.png"
    else
        echo "Skipping system-wide icon install."
    fi
fi

# Update user icon cache
gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

# 3. Configure Desktop Entry
echo "[3/4] Configuring desktop entry..."

# Create a temporary desktop file
cat > "setup/$DESKTOP_FILE.tmp" <<EOF
[Desktop Entry]
Name=Immich Sync
Comment=Automatically upload photos to Immich
Exec=env XDG_CURRENT_DESKTOP=Unity GDK_BACKEND=x11 "$PYTHON_EXEC" "$SCRIPT_PATH"
Icon=$ICON_NAME
Terminal=false
Type=Application
Categories=Utility;Network;
Keywords=Photo;Sync;Backup;
StartupNotify=false
StartupWMClass=immich-sync.desktop
EOF

# Install Desktop Entry
mv "setup/$DESKTOP_FILE.tmp" "$USER_APPS/$DESKTOP_FILE"
# Make it executable just in case
chmod +x "$USER_APPS/$DESKTOP_FILE"
echo "Installed $DESKTOP_FILE to $USER_APPS"

# 4. Autostart Configuration
echo "[4/4] Finalizing..."
read -p "Do you want to start automatically on login? (y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    ln -sf "$USER_APPS/$DESKTOP_FILE" "$AUTOSTART_DIR/$DESKTOP_FILE"
    echo "Autostart enabled."
else
    rm -f "$AUTOSTART_DIR/$DESKTOP_FILE"
    echo "Autostart disabled."
fi

echo "=== Installation Complete! ==="
echo "You may need to log out and back in for the icon to appear in your menu."
