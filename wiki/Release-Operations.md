# Release Operations

This page is for maintainers.

## Version Bumps

For a normal release, update:

- `Cargo.toml`
- `Cargo.lock`
- `CHANGELOG.md`
- `io.github.nicx17.mimick.yml`
- `setup/metainfo/io.github.nicx17.mimick.metainfo.xml`

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

## Release Flow

1. land the release commit on `main`
2. create an annotated tag such as `v6.0.0`
3. push `main` and the tag
4. verify the release workflow and Flatpak Pages workflow complete successfully

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
