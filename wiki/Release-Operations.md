# Release Operations

This page is for maintainers.

## Version Bumps

For a normal release, update:

- `Cargo.toml`
- `Cargo.lock`
- `CHANGELOG.md`
- `dev.nicx.mimick.yml`
- `setup/metainfo/dev.nicx.mimick.metainfo.xml`

## Signed Distribution Outputs

Mimick publishes:

- a GitHub Pages Flatpak repository
- GitHub release assets including `mimick.flatpakrepo` and `SHA256SUMS.txt`

Both publication flows use the Flatpak signing key configured in GitHub Actions secrets.

Current published Flatpak repo signing fingerprint:

`04E2 9556 E951 B2EA 15D3 A8EE 632E 1BC5 D956 579C`

## Required Secrets

- `FLATPAK_GPG_PRIVATE_KEY`
- `FLATPAK_GPG_KEY_ID`
- `FLATPAK_GPG_PASSPHRASE` if the key is protected

## Pre-Flight Validation

Before tagging a release, ensure all metadata, manifests, and licenses are updated and valid:

```bash
# Validate Desktop Entry and AppStream Metadata
desktop-file-validate setup/dev.nicx.mimick.desktop
appstreamcli validate --explain setup/metainfo/dev.nicx.mimick.metainfo.xml

# Lint Flatpak manifest and build output
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest dev.nicx.mimick.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder builddir build-dir # Run after a successful build

# Check code quality and formatting
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings

# Refresh Flatpak Cargo sources
python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json

# Update third-party license summaries
cargo about generate about.hbs --output-file THIRD_PARTY_LICENSES.txt
cargo about generate about-summary.hbs --output-file THIRD_PARTY_LICENSES_SUMMARY.txt
```

## Release Flow

1. land the release-prep commit on `main`
2. make sure `Cargo.toml`, `CHANGELOG.md`, the Flatpak manifest tag, and AppStream metadata all agree on the version
3. create an annotated tag such as `v8.0.0`
4. push `main` and the tag
5. verify the release workflow and Flatpak Pages workflow complete successfully

The release workflow expects the tag version to exactly match the crate version in `Cargo.toml`.

## Re-Releasing a Version

If a workflow issue needs a tag rerun:

1. fix the workflow on `main`
2. delete the old tag locally and remotely
3. recreate the annotated tag on the corrected commit
4. push the tag again

## Key Rotation

If the Flatpak signing key changes:

1. update the GitHub secrets
2. confirm the generated `.flatpakrepo` publishes the new public key material
3. update the published fingerprint in `README.md` and the wiki installation/release pages
4. call out the key rotation in release notes so users know the change was intentional
