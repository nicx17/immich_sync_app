# Installation

The recommended install method is the official Flatpak repository.

## Flatpak

```bash
flatpak remote-add --user --if-not-exists mimick-repo https://nicx17.github.io/mimick/mimick.flatpakrepo
flatpak install --user mimick-repo io.github.nicx17.mimick
```

## Verify the Repo Signing Key

The Mimick Flatpak repository currently publishes this signing-key fingerprint:

`04E2 9556 E951 B2EA 15D3 A8EE 632E 1BC5 D956 579C`

To inspect the key embedded in the live `.flatpakrepo` file:

```bash
curl -fsSL https://nicx17.github.io/mimick/mimick.flatpakrepo \
  | sed -n 's/^GPGKey=//p' \
  | base64 -d > /tmp/mimick-repo-public.gpg

gpg --show-keys --fingerprint /tmp/mimick-repo-public.gpg
```

Compare the resulting fingerprint to the value above. Treat the fingerprint, not the email address on the key, as the identity marker.

Run the app with:

```bash
flatpak run io.github.nicx17.mimick
```

Open the settings window directly with:

```bash
flatpak run io.github.nicx17.mimick --settings
```

## Local Development Build

For a native development run:

```bash
cargo run
```

Open settings immediately:

```bash
cargo run -- --settings
```

For a local Flatpak build that uses the current checkout instead of the GitHub source tag:

```bash
flatpak-builder --user --install --force-clean build-dir io.github.nicx17.mimick.local.yml
```

## What Gets Installed

- Application ID: `io.github.nicx17.mimick`
- Binary: `mimick`
- Config file: `~/.config/mimick/config.json`
- Cache directory: `~/.cache/mimick/`
