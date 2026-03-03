#!/bin/bash
# uninstall-appimage.sh - Removes the Immich Sync AppImage for the current user

APP_NAME="immich-sync"
USER_BIN="$HOME/.local/bin"
USER_APPS="$HOME/.local/share/applications"
USER_ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
TARGET_APPIMAGE="$USER_BIN/immich-sync.AppImage"
AUTOSTART_DIR="$HOME/.config/autostart"

echo "=== Uninstalling Immich Sync AppImage ==="

if [ -f "$TARGET_APPIMAGE" ]; then
    rm "$TARGET_APPIMAGE"
    echo "Removed executable."
fi

if [ -f "$USER_APPS/$APP_NAME.desktop" ]; then
    rm "$USER_APPS/$APP_NAME.desktop"
    echo "Removed desktop entry."
fi

if [ -f "$AUTOSTART_DIR/$APP_NAME.desktop" ]; then
    rm "$AUTOSTART_DIR/$APP_NAME.desktop"
    echo "Removed autostart entry."
fi

if [ -f "$USER_ICONS/$APP_NAME.png" ]; then
    rm "$USER_ICONS/$APP_NAME.png"
    echo "Removed icon."
    gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

echo "Uninstallation Complete."
