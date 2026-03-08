#!/bin/bash
# uninstall-appimage.sh - Removes the Mimick AppImage for the current user

APP_NAME="mimick"
USER_BIN="$HOME/.local/bin"
USER_APPS="$HOME/.local/share/applications"
USER_ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
USER_ICONS_SCALABLE="$HOME/.local/share/icons/hicolor/scalable/apps"
TARGET_APPIMAGE="$USER_BIN/mimick.AppImage"
AUTOSTART_DIR="$HOME/.config/autostart"

echo "=== Uninstalling Mimick AppImage ==="

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
    echo "Removed PNG icon."
fi

if [ -f "$USER_ICONS_SCALABLE/$APP_NAME.svg" ]; then
    rm "$USER_ICONS_SCALABLE/$APP_NAME.svg"
    echo "Removed SVG icon."
fi

gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Uninstallation Complete."
