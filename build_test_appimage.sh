#!/bin/bash
set -e

echo "Cleaning up old build artifacts..."
rm -rf AppDir squashfs-root appimagetool-x86_64.AppImage Mimick-*.AppImage python-standalone.tar.gz

echo "Downloading AppImageTool..."
wget -q -c "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x appimagetool-x86_64.AppImage

echo "Creating AppDir structure..."
mkdir -p AppDir/usr/bin
mkdir -p AppDir/usr/share/icons/hicolor/256x256/apps
mkdir -p AppDir/usr/share/metainfo
mkdir -p AppDir/usr/share/applications

echo "Copying source and assets..."
cp -r src/* AppDir/usr/bin/
mv AppDir/usr/bin/main.py AppDir/usr/bin/mimick
chmod +x AppDir/usr/bin/mimick

cp src/assets/icon.png AppDir/mimick.png
cp src/assets/icon.png AppDir/usr/share/icons/hicolor/256x256/apps/mimick.png
mkdir -p AppDir/usr/share/icons/hicolor/scalable/apps
cp src/assets/icon.svg AppDir/usr/share/icons/hicolor/scalable/apps/mimick.svg
cp setup/metainfo/com.nickcardoso.mimick.appdata.xml AppDir/usr/share/metainfo/com.nickcardoso.mimick.appdata.xml 2>/dev/null || true

echo "Downloading Standalone Portable Python (3.12)..."
# We download a self-contained Python runtime so we never depend on the host OS Python version.
wget -q -c "https://github.com/astral-sh/python-build-standalone/releases/download/20260303/cpython-3.12.13%2B20260303-x86_64-unknown-linux-gnu-install_only.tar.gz" -O python-standalone.tar.gz
mkdir -p AppDir/usr/python
tar -xzf python-standalone.tar.gz -C AppDir/usr/python --strip-components=1

echo "Installing dependencies into the portable Python..."
# We use the bundled python to ensure binary compatibility
AppDir/usr/python/bin/python3 -m pip install -r requirements.txt

echo "Creating Desktop Entry..."
cat << 'APP_EOF' > AppDir/com.nickcardoso.mimick.desktop
[Desktop Entry]
Name=Mimick
Exec=mimick
Icon=mimick
Type=Application
Categories=Utility;Network;
Comment=Automatic background sync for Immich
Terminal=false
Actions=Settings;

[Desktop Action Settings]
Name=Open Settings
Exec=mimick --settings
APP_EOF
cp AppDir/com.nickcardoso.mimick.desktop AppDir/usr/share/applications/com.nickcardoso.mimick.desktop

echo "Creating AppRun..."
cat << 'APP_EOF' > AppDir/AppRun
#!/bin/sh

export HERE="$(dirname "$(readlink -f "${0}")")"
# Force the system to use our bundled Python and ignore the host's Python 
export PATH="$HERE/usr/python/bin:$PATH"
export PYTHONPATH="$HERE/usr/bin:$PYTHONPATH"
# Ensure PyGObject can find the host system's GTK and AppIndicator typelib bindings
export GI_TYPELIB_PATH="/usr/lib/girepository-1.0:/usr/lib/x86_64-linux-gnu/girepository-1.0:/usr/lib64/girepository-1.0:\$GI_TYPELIB_PATH"
exec "$HERE/usr/python/bin/python3" "$HERE/usr/bin/mimick" "$@"
APP_EOF
chmod +x AppDir/AppRun

echo "Building AppImage..."
# Extract version from pyproject.toml
VERSION=$(grep 'version = ' pyproject.toml | cut -d '"' -f 2)
echo "Found version: $VERSION"

VERSION=$VERSION ARCH=x86_64 ./appimagetool-x86_64.AppImage AppDir
chmod +x Mimick-$VERSION-x86_64.AppImage

echo "Build complete! Cleaning up tool and temp files..."
rm -rf AppDir appimagetool-x86_64.AppImage python-standalone.tar.gz
