# Security Policy

## Supported Versions

Security fixes are applied to the latest released version on `main`.

Older releases are not guaranteed to receive backported fixes.

## Reporting a Vulnerability

Please do not report sensitive security issues in public GitHub issues.

Preferred reporting path:

1. Use GitHub's private vulnerability reporting for this repository if it is enabled.
2. If private reporting is unavailable, contact the maintainer through a trusted private channel first.

When reporting a vulnerability, include:

- the Mimick version
- how it was installed (`Flatpak`, local build, etc.)
- the affected operating system and desktop environment
- reproduction steps or a proof of concept
- whether the issue can expose files, API keys, or remote account access

## Security Practices in This Project

Mimick already uses several controls intended to reduce risk:

- Flatpak distribution is signed with a dedicated repository signing key
- release assets include checksums
- the API key is stored in the desktop keyring instead of plain-text config
- Flatpak builds use selected-folder access instead of broad home-directory access
- CodeQL is enabled for static analysis
- CI enforces formatting, linting, tests, and dependency auditing

## Flatpak Repo Signing Fingerprint

Current published Flatpak repo signing fingerprint:

`04E2 9556 E951 B2EA 15D3 A8EE 632E 1BC5 D956 579C`

You can verify the key embedded in the live `.flatpakrepo` file with:

```bash
curl -fsSL https://mimick.nicx.dev/mimick.flatpakrepo \
  | sed -n 's/^GPGKey=//p' \
  | base64 -d > /tmp/mimick-repo-public.gpg

gpg --show-keys --fingerprint /tmp/mimick-repo-public.gpg
```

Compare the printed fingerprint to the value above. The fingerprint is the trust anchor, not the email address on the key.
