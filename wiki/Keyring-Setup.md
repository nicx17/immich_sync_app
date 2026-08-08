# Keyring Setup

Mimick stores your Immich API key securely in your desktop's keyring.
On **GNOME** and **KDE** this works out of the box.
On other desktops -- **Hyprland**, **Sway**, **XFCE**, **i3**, **Cosmic**,
and similar compositors -- the keyring portal may not be wired up by
default, causing the error:

> Could Not Save API Key

This page explains how to fix it.

---

## Background

Mimick uses the [oo7](https://crates.io/crates/oo7) keyring library,
which stores credentials through one of two backends:

1. **Portal backend** (Flatpak sandbox) --
   uses `org.freedesktop.portal.Secret`, exposed by
   `gnome-keyring` or `kwallet`.
2. **D-Bus Secret Service** (native installs) --
   uses `org.freedesktop.secrets`, the standard Linux Secret
   Service API.

Both backends need a keyring daemon *and* correct portal wiring.
GNOME and KDE ship `.portal` files that register their keyring
daemons automatically (`UseIn=gnome` / `UseIn=kde`).
On other compositors, `xdg-desktop-portal` skips those files, so
the Secret portal is never exposed.

---

## Prerequisites

Regardless of your desktop, ensure these are installed:

- **gnome-keyring** (or **kwallet** if you prefer the KDE stack)
- **libsecret**
- **xdg-desktop-portal**
- Your compositor's portal backend (e.g., `xdg-desktop-portal-hyprland`,
  `xdg-desktop-portal-wlr`, `xdg-desktop-portal-gtk`)

Confirm the keyring daemon is running:

```bash
systemctl --user status gnome-keyring-daemon
```

---

## Fix by Desktop

### Hyprland / Sway / wlroots compositors

Create (or edit) `~/.config/xdg-desktop-portal/portals.conf`:

```ini
[preferred]
default=hyprland;gtk
org.freedesktop.impl.portal.Secret=gnome-keyring
```

> Replace `hyprland` with `wlr` or your compositor's portal name.

Then restart the portal:

```bash
systemctl --user restart xdg-desktop-portal
```

### XFCE

1. Install `xdg-desktop-portal-xapp` (in addition to
   `xdg-desktop-portal-gtk`).
2. Edit `/usr/share/xdg-desktop-portal/xfce-portals.conf`:

```ini
[preferred]
default=xapp;gtk;
```

3. **Remove** any overriding files that may conflict:
   - `~/.config/xdg-desktop-portal/portals.conf`
   - `/etc/xdg-desktop-portal/portals.conf`

4. Restart the portal:

```bash
systemctl --user restart xdg-desktop-portal
```

### i3 / other X11 window managers

Create `~/.config/xdg-desktop-portal/portals.conf`:

```ini
[preferred]
default=gtk
org.freedesktop.impl.portal.Secret=gnome-keyring
```

Then restart:

```bash
systemctl --user restart xdg-desktop-portal
```

### Cosmic

If you are using the COSMIC desktop, install the COSMIC portal
backend and configure:

```ini
[preferred]
default=cosmic;gtk
org.freedesktop.impl.portal.Secret=gnome-keyring
```

---

## Verifying the Fix

After restarting the portal, test that the Secret portal is exposed:

```bash
busctl --user call org.freedesktop.portal.Desktop \
  /org/freedesktop/portal/desktop \
  org.freedesktop.DBus.Properties Get \
  ss org.freedesktop.portal.Secret version
```

A successful result returns the portal version.
If it errors with "Unknown interface", the portal is still not configured.

Then relaunch Mimick and try saving your API key again.

---

## Still Not Working?

If you have configured the portal but the error persists:

1. Check logs: run Mimick from a terminal to see detailed keyring errors.
2. Ensure gnome-keyring has an unlocked "default" collection:

```bash
secret-tool store --label='test' test-key test-value
secret-tool lookup test-key test-value
```

3. File an issue at
   [github.com/nicx17/mimick/issues](https://github.com/nicx17/mimick/issues)
   with the error output from step 1.
