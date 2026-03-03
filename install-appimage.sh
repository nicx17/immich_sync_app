#!/bin/bash
# install-appimage.sh - Installs the Immich Sync AppImage for the current user

set -e

APP_NAME="immich-sync"
APPIMAGE_SRC="Immich_Sync-x86_64.AppImage"
USER_BIN="$HOME/.local/bin"
USER_APPS="$HOME/.local/share/applications"
USER_ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
TARGET_APPIMAGE="$USER_BIN/immich-sync.AppImage"
AUTOSTART_DIR="$HOME/.config/autostart"

if [ ! -f "$APPIMAGE_SRC" ]; then
    echo "Error: $APPIMAGE_SRC not found in current directory."
    echo "Please build the AppImage first."
    exit 1
fi

echo "=== Installing Immich Sync AppImage ==="

# 1. Create directories
mkdir -p "$USER_BIN"
mkdir -p "$USER_APPS"
mkdir -p "$USER_ICONS"
mkdir -p "$AUTOSTART_DIR"

# 2. Copy AppImage
echo "Installing executable to $USER_BIN..."
cp "$APPIMAGE_SRC" "$TARGET_APPIMAGE"
chmod +x "$TARGET_APPIMAGE"

# 3. Extract and install icon
echo "Extracting icon..."
# The AppImage is an executable that can extract itself if told to
# But we already have the source, so we'll just use the source icon for ease
cp src/assets/icon.png "$USER_ICONS/$APP_NAME.png"
gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

# 4. Create Desktop Entry
echo "Creating desktop entry..."
cat > "$USER_APPS/$APP_NAME.desktop" <<DESKTOP
[Desktop Entry]
Name=Immich Sync
Comment=Automatic background sync for Immich
Exec=$TARGET_APPIMAGE
Icon=$APP_NAME
Terminal=false
Type=Application
Categories=Utility;Network;
StartupNotify=false
StartupWMClass=immich-sync
DESKTOP
chmod +x "$USER_APPS/$APP_NAME.desktop"

# 5. Autostart check
read -p "Do you want to start automatically on login? (y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    ln -sf "$USER_APPS/$APP_NAME.desktop" "$AUTOSTART_DIR/$APP_NAME.desktop"
    echo "Autostart enabled."
else
    rm -f "$AUTOSTART_DIR/$APP_NAME.desktop"
    echo "Autostart disabled."
fi

# Ensure ~/.local/bin is in PATH for terminal use
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo ""
    echo "Note: $HOME/.local/bin is not in your PATH."
    echo "Add 'export PATH=\"\$HOME/.local/bin:\$PATH\"' to your ~/.bashrc or ~/.zshrc."
fi

echo "=== Installation Complete! ==="
echo "You can launch 'Immich Sync' from your application menu."
