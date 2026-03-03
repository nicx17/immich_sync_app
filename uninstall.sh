#!/bin/bash
# uninstall.sh - Remove Immich Sync integration

APP_NAME="immich-sync"
ICON_NAME="immich-sync"
DESKTOP_FILE="immich-sync.desktop"

# Directories
USER_APPS="$HOME/.local/share/applications"
USER_ICONS_SCALABLE="$HOME/.local/share/icons/hicolor/scalable/apps"
USER_ICONS_PNG="$HOME/.local/share/icons/hicolor/128x128/apps"
AUTOSTART_DIR="$HOME/.config/autostart"

echo "Uninstalling $APP_NAME..."

# 1. Remove Desktop Entry
if [ -f "$USER_APPS/$DESKTOP_FILE" ]; then
    rm "$USER_APPS/$DESKTOP_FILE"
    echo "Removed desktop entry: $USER_APPS/$DESKTOP_FILE"
else
    echo "Desktop entry not found."
fi

# 2. Remove Autostart Entry
if [ -f "$AUTOSTART_DIR/$DESKTOP_FILE" ]; then
    rm "$AUTOSTART_DIR/$DESKTOP_FILE"
    echo "Removed autostart entry: $AUTOSTART_DIR/$DESKTOP_FILE"
else
    echo "Autostart entry not found."
fi

# 3. Remove Icons
# SVG
if [ -f "$USER_ICONS_SCALABLE/$ICON_NAME.svg" ]; then
    rm "$USER_ICONS_SCALABLE/$ICON_NAME.svg"
    echo "Removed icon: $USER_ICONS_SCALABLE/$ICON_NAME.svg"
fi
# PNG
if [ -f "$USER_ICONS_PNG/$ICON_NAME.png" ]; then
    rm "$USER_ICONS_PNG/$ICON_NAME.png"
    echo "Removed icon: $USER_ICONS_PNG/$ICON_NAME.png"
fi
# System-wide (check if exists)
if [ -f "/usr/share/pixmaps/$ICON_NAME.png" ]; then
    echo "Found system-wide icon at /usr/share/pixmaps/$ICON_NAME.png"
    read -p "Remove system-wide icon? (requires sudo) (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        sudo rm "/usr/share/pixmaps/$ICON_NAME.png"
        echo "Removed /usr/share/pixmaps/$ICON_NAME.png"
    fi
fi

# Update icon cache
gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

# 4. Cleanup Environment
read -p "Remove virtual environment (.venv)? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    #rm -rf .venv
    echo "Removed virtual environment."
fi

echo "Uninstallation Complete."
