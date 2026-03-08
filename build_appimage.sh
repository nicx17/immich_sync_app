#!/bin/bash
# build_appimage.sh - Build a self-contained AppImage for Mimick (Rust binary)
set -e

# Extract version from Cargo.toml
VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d '"' -f2)
echo "Building Mimick $VERSION AppImage..."

# Clean previous artefacts
rm -rf AppDir appimagetool-x86_64.AppImage "Mimick-$VERSION-x86_64.AppImage"

# 1. Build optimised release binary
echo "[1/5] Building release binary..."
cargo build --release
BINARY="target/release/mimick"
if [ ! -f "$BINARY" ]; then
    echo "Error: build failed — $BINARY not found."
    exit 1
fi

# 2. Download AppImageTool
echo "[2/5] Downloading appimagetool..."
wget -q -c "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x appimagetool-x86_64.AppImage

# 3. Assemble AppDir
echo "[3/5] Assembling AppDir..."
mkdir -p AppDir/usr/bin
mkdir -p AppDir/usr/share/applications
mkdir -p AppDir/usr/share/icons/hicolor/256x256/apps
mkdir -p AppDir/usr/share/icons/hicolor/scalable/apps
mkdir -p AppDir/usr/share/metainfo

# Binary
cp "$BINARY" AppDir/usr/bin/mimick
chmod +x AppDir/usr/bin/mimick

# Icons
cp src/assets/icon.png AppDir/mimick.png
cp src/assets/icon.png AppDir/usr/share/icons/hicolor/256x256/apps/mimick.png
[ -f "src/assets/icon.svg" ] && cp src/assets/icon.svg AppDir/usr/share/icons/hicolor/scalable/apps/mimick.svg
[ -f "setup/metainfo/com.nickcardoso.mimick.appdata.xml" ] && \
    cp setup/metainfo/com.nickcardoso.mimick.appdata.xml AppDir/usr/share/metainfo/

# Desktop entry
cat > AppDir/com.nickcardoso.mimick.desktop << 'EOF'
[Desktop Entry]
Name=Mimick
Comment=Automatically upload photos to Immich
Exec=mimick
Icon=mimick
Type=Application
Categories=Utility;Network;
Terminal=false
Keywords=Photo;Sync;Backup;Immich;

[Desktop Action Settings]
Name=Open Settings
Exec=mimick --settings
EOF
cp AppDir/com.nickcardoso.mimick.desktop AppDir/usr/share/applications/com.nickcardoso.mimick.desktop

# AppRun — the GTK4 libraries must come from the host system; do not bundle them
cat > AppDir/AppRun << 'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/mimick" "$@"
EOF
chmod +x AppDir/AppRun

# 4. Build AppImage
echo "[4/5] Packaging AppImage..."
VERSION="$VERSION" ARCH=x86_64 ./appimagetool-x86_64.AppImage AppDir "Mimick-$VERSION-x86_64.AppImage"
chmod +x "Mimick-$VERSION-x86_64.AppImage"

# 5. Cleanup
echo "[5/5] Cleaning up..."
rm -rf AppDir appimagetool-x86_64.AppImage

echo ""
echo "Done: Mimick-$VERSION-x86_64.AppImage"
