#!/bin/bash
set -e

echo "Cleaning up old build artifacts..."
rm -rf AppDir squashfs-root appimagetool-x86_64.AppImage Immich_Sync-x86_64.AppImage

echo "Downloading AppImageTool..."
wget -q -c "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x appimagetool-x86_64.AppImage

echo "Creating AppDir structure..."
mkdir -p AppDir/usr/bin
mkdir -p AppDir/usr/lib/python3/site-packages
mkdir -p AppDir/usr/share/icons/hicolor/256x256/apps

echo "Copying source and assets..."
cp -r src/* AppDir/usr/bin/
mv AppDir/usr/bin/main.py AppDir/usr/bin/immich-sync
chmod +x AppDir/usr/bin/immich-sync

cp src/assets/icon.png AppDir/immich-sync.png
cp src/assets/icon.png AppDir/usr/share/icons/hicolor/256x256/apps/immich-sync.png

echo "Installing dependencies..."
pip install -r requirements.txt --target=AppDir/usr/lib/python3/site-packages

echo "Creating Desktop Entry..."
cat << 'EOF' > AppDir/immich-sync.desktop
[Desktop Entry]
Name=Immich Sync
Exec=immich-sync
Icon=immich-sync
Type=Application
Categories=Utility;Network;
Comment=Automatic background sync for Immich
Terminal=false
EOF

echo "Creating AppRun..."
cat << 'EOF' > AppDir/AppRun
#!/bin/sh

export HERE="$(dirname "$(readlink -f "${0}")")"
export PYTHONPATH="$HERE/usr/lib/python3/site-packages:$PYTHONPATH"

exec python3 "$HERE/usr/bin/immich-sync" "$@"
EOF
chmod +x AppDir/AppRun

echo "Building AppImage..."
ARCH=x86_64 ./appimagetool-x86_64.AppImage AppDir
chmod +x Immich_Sync-x86_64.AppImage

echo "Build complete! Cleaning up tool..."
rm -rf AppDir appimagetool-x86_64.AppImage