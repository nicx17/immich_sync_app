#!/bin/bash
# install.sh - Build and install Mimick from source (Rust)

set -e

APP_NAME="mimick"
ICON_NAME="mimick"
DESKTOP_FILE="mimick.desktop"

# Directories
INSTALL_BIN="$HOME/.local/bin"
USER_APPS="$HOME/.local/share/applications"
USER_ICONS_SCALABLE="$HOME/.local/share/icons/hicolor/scalable/apps"
USER_ICONS_PNG="$HOME/.local/share/icons/hicolor/128x128/apps"
AUTOSTART_DIR="$HOME/.config/autostart"

mkdir -p "$INSTALL_BIN" "$USER_APPS" "$USER_ICONS_SCALABLE" "$USER_ICONS_PNG" "$AUTOSTART_DIR"

echo "=== Mimick Installer (Rust) ==="

# 1. Check prerequisites
echo "[1/4] Checking prerequisites..."

if ! command -v cargo &> /dev/null; then
    echo "Error: Rust toolchain (cargo) not found."
    echo "Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check for required GTK4 / Adwaita system libraries
MISSING_PKGS=()
for lib in "gtk4" "libadwaita-1"; do
    if ! pkg-config --exists "$lib" 2>/dev/null; then
        MISSING_PKGS+=("$lib")
    fi
done

if [ ${#MISSING_PKGS[@]} -gt 0 ]; then
    echo "Missing system libraries: ${MISSING_PKGS[*]}"
    echo ""
    echo "Install them first:"
    echo "  Ubuntu/Debian: sudo apt install libgtk-4-dev libadwaita-1-dev libglib2.0-dev pkg-config build-essential"
    echo "  Fedora:        sudo dnf install gtk4-devel libadwaita-devel"
    echo "  Arch Linux:    sudo pacman -S gtk4 libadwaita"
    exit 1
fi

# 2. Build release binary
echo "[2/4] Building release binary..."
cargo build --release
BINARY_PATH="$(pwd)/target/release/$APP_NAME"

if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Build failed — binary not found at $BINARY_PATH"
    exit 1
fi

# Install binary to ~/.local/bin
install -Dm755 "$BINARY_PATH" "$INSTALL_BIN/$APP_NAME"
echo "Installed binary to $INSTALL_BIN/$APP_NAME"

# 3. Install icons
echo "[3/4] Installing icons..."

if [ -f "setup/icons/mimick.svg" ]; then
    cp "setup/icons/mimick.svg" "$USER_ICONS_SCALABLE/$ICON_NAME.svg"
fi

if [ -f "src/assets/icon.png" ]; then
    cp "src/assets/icon.png" "$USER_ICONS_PNG/$ICON_NAME.png"

    if [ -w "/usr/share/pixmaps" ]; then
        cp "src/assets/icon.png" "/usr/share/pixmaps/$ICON_NAME.png"
    else
        read -p "Install icon system-wide to /usr/share/pixmaps? (requires sudo) (y/N) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            sudo cp "src/assets/icon.png" "/usr/share/pixmaps/$ICON_NAME.png"
        fi
    fi
fi

gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

# 4. Desktop entry and autostart
echo "[4/4] Configuring desktop entry..."

cat > "$USER_APPS/$DESKTOP_FILE" << EOF
[Desktop Entry]
Name=Mimick
Comment=Automatically upload photos to Immich
Exec=$INSTALL_BIN/$APP_NAME
Icon=$ICON_NAME
Terminal=false
Type=Application
Categories=Utility;Network;
Keywords=Photo;Sync;Backup;
StartupNotify=false

[Desktop Action Settings]
Name=Open Settings
Exec=$INSTALL_BIN/$APP_NAME --settings
EOF

chmod +x "$USER_APPS/$DESKTOP_FILE"
echo "Installed $DESKTOP_FILE to $USER_APPS"

read -p "Enable autostart on login? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    ln -sf "$USER_APPS/$DESKTOP_FILE" "$AUTOSTART_DIR/$DESKTOP_FILE"
    echo "Autostart enabled."
else
    rm -f "$AUTOSTART_DIR/$DESKTOP_FILE"
    echo "Autostart disabled."
fi

echo ""
echo "=== Installation Complete ==="
echo "Run with: $APP_NAME"
echo "Or open from your Application Launcher."
