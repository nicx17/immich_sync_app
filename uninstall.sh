#!/bin/bash
# uninstall.sh - Remove Mimick (Rust binary installation)

APP_NAME="mimick"
ICON_NAME="mimick"
DESKTOP_FILE="mimick.desktop"

INSTALL_BIN="$HOME/.local/bin"
USER_APPS="$HOME/.local/share/applications"
USER_ICONS_SCALABLE="$HOME/.local/share/icons/hicolor/scalable/apps"
USER_ICONS_PNG="$HOME/.local/share/icons/hicolor/128x128/apps"
AUTOSTART_DIR="$HOME/.config/autostart"

echo "=== Mimick Uninstaller ==="

# 1. Remove binary
if [ -f "$INSTALL_BIN/$APP_NAME" ]; then
    rm "$INSTALL_BIN/$APP_NAME"
    echo "Removed binary: $INSTALL_BIN/$APP_NAME"
else
    echo "Binary not found at $INSTALL_BIN/$APP_NAME"
fi

# 2. Remove desktop entry
if [ -f "$USER_APPS/$DESKTOP_FILE" ]; then
    rm "$USER_APPS/$DESKTOP_FILE"
    echo "Removed desktop entry: $USER_APPS/$DESKTOP_FILE"
fi

# 3. Remove autostart entry
if [ -f "$AUTOSTART_DIR/$DESKTOP_FILE" ]; then
    rm "$AUTOSTART_DIR/$DESKTOP_FILE"
    echo "Removed autostart entry."
fi

# 4. Remove icons
[ -f "$USER_ICONS_SCALABLE/$ICON_NAME.svg" ] && rm "$USER_ICONS_SCALABLE/$ICON_NAME.svg" && echo "Removed SVG icon."
[ -f "$USER_ICONS_PNG/$ICON_NAME.png" ]      && rm "$USER_ICONS_PNG/$ICON_NAME.png"      && echo "Removed PNG icon."

if [ -f "/usr/share/pixmaps/$ICON_NAME.png" ]; then
    read -p "Remove system-wide icon from /usr/share/pixmaps? (requires sudo) (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        sudo rm "/usr/share/pixmaps/$ICON_NAME.png"
        echo "Removed system-wide icon."
    fi
fi

gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

# 5. Optionally remove config + cache
read -p "Remove config (~/.config/mimick) and cache (~/.cache/mimick)? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    rm -rf "$HOME/.config/mimick"
    rm -rf "$HOME/.cache/mimick"
    echo "Removed config and cache directories."
fi

echo ""
echo "=== Uninstallation Complete ==="
