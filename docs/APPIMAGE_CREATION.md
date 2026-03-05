# Building the Immich Sync AppImage

This document details the process for manually building an AppImage for the Immich Sync application. This packages the application along with its Python dependencies into a single, portable executable file that doesn't rely on the system's `site-packages`.

## Prerequisites

1. A Linux environment (tested on x86_64).
2. `wget` and standard GNU utilities (`cp`, `mkdir`, `chmod`).
3. System Python 3 installed.
4. `pip` installed (for fetching project dependencies).

---

## Step 1: Prepare the Build Tool

We use the official `appimagetool` to generate the final AppImage.

```bash
# Download the continuous build of AppImageTool
wget -c "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"

# Make it executable
chmod +x appimagetool-x86_64.AppImage
```

---

## Step 2: Create the `AppDir` Structure

AppImages are built from a specifically structured folder named `AppDir`.

```bash
# Create the standard directory tree
mkdir -p AppDir/usr/bin
mkdir -p AppDir/usr/lib/python3/site-packages
mkdir -p AppDir/usr/share/icons/hicolor/256x256/apps
```

---

## Step 3: Migrate Assets and Code

Copy the application source code and graphical assets into the standard locations inside the `AppDir`.

```bash
# Copy source code and set permissions
cp -r src/* AppDir/usr/bin/
mv AppDir/usr/bin/main.py AppDir/usr/bin/immich-sync
chmod +x AppDir/usr/bin/immich-sync

# Copy icons (root location for AppImage standard, standard location for system)
cp src/assets/icon.png AppDir/immich-sync.png
cp src/assets/icon.png AppDir/usr/share/icons/hicolor/256x256/apps/immich-sync.png
```

---

## Step 4: Install Dependencies

We must explicitly install python dependencies into the isolated `AppDir` to ensure they are shipped internally and the app won't crash if the host system lacks them.

```bash
# Target the built-in system packages folder
pip install -r requirements.txt --target=AppDir/usr/lib/python3/site-packages
```

---

## Step 5: Define application metadata

Create the standard Desktop entry inside the root of the `AppDir`. This gives the AppImage a name, an icon, and instructions on what to run.

```bash
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
```

---

## Step 6: Create the `AppRun` Hook

`AppRun` is the entry point for the AppImage. It calculates where the AppImage was mounted, injects the internal site-packages into the `PYTHONPATH`, and starts the main script using the host's system Python runtime.

```bash
cat << 'EOF' > AppDir/AppRun
#!/bin/sh

# Define the absolute path to the AppImage contents
export HERE="$(dirname "$(readlink -f "${0}")")"

# Force Python to look in our bundled directory first
export PYTHONPATH="$HERE/usr/lib/python3/site-packages:$PYTHONPATH"

# Execute the python script using the system's python3 interpreter
exec python3 "$HERE/usr/bin/immich-sync" "$@"
EOF

# Make the hook executable
chmod +x AppDir/AppRun
```

---

## Step 7: Build the AppImage

Invoke the `appimagetool` to compress the `AppDir` directory into a monolithic `.AppImage` file using `squashfs`.

```bash
# Build the AppImage by extracting the version from pyproject.toml first (Force architecture if needed)
VERSION=$(grep 'version = ' pyproject.toml | cut -d '"' -f 2)
VERSION=$VERSION ARCH=x86_64 ./appimagetool-x86_64.AppImage AppDir

# Rename for cleanliness if necessary, set permissions 
chmod +x Immich_Sync-$VERSION-x86_64.AppImage
```

---

## Step 8: Clean Up

Remove the temporary utilities and directories.

```bash
rm -rf AppDir
rm appimagetool-x86_64.AppImage
```

The resulting `Immich_Sync-<VERSION>-x86_64.AppImage` is now ready for distribution.
