# Repository Automation

Mimick uses a small, focused automation stack for repository hygiene and release safety.

## Current Automation

- Dependabot for Cargo and GitHub Actions updates
- CODEOWNERS for default repository ownership
- Maintainer approval gate for PRs opened by other contributors
- Cargo vendor guard for `cargo-sources.json`
- Docs link checking for README, docs, and wiki pages
- Release Drafter for a rolling draft release page
- Dependabot auto-merge workflow for approved dependency PRs

## Why It Exists

This repo ships a Flatpak build that depends on vendored Cargo metadata, maintains a manual changelog, and relies on workflow-driven releases. A few guardrails go a long way here.

## Important Manual Settings

The important GitHub-side settings are now in place on `main`:

1. `Allow auto-merge`
2. required status checks:
   `Format, Lint, and Test`, `Dependency Audit`, and `Verify cargo-sources.json`
3. 1 required approving review
4. required code-owner review
5. stale approvals are dismissed on new commits
6. conversation resolution is required

Because the repo-wide CODEOWNERS entry points to `@nicx17`, PRs from other contributors now need your approval before merging.

Admins are not enforced, so your own PRs remain practical to merge when needed.

## Key Files

- [`.github/dependabot.yml`](https://github.com/nicx17/mimick/blob/main/.github/dependabot.yml)
- [`.github/CODEOWNERS`](https://github.com/nicx17/mimick/blob/main/.github/CODEOWNERS)
- [`.github/workflows/maintainer-approval.yml`](https://github.com/nicx17/mimick/blob/main/.github/workflows/maintainer-approval.yml)
- [`.github/workflows/cargo-sources-guard.yml`](https://github.com/nicx17/mimick/blob/main/.github/workflows/cargo-sources-guard.yml)
- [`.github/workflows/docs-links.yml`](https://github.com/nicx17/mimick/blob/main/.github/workflows/docs-links.yml)
- [`.github/workflows/release-drafter.yml`](https://github.com/nicx17/mimick/blob/main/.github/workflows/release-drafter.yml)
- [`.github/workflows/dependabot-auto-merge.yml`](https://github.com/nicx17/mimick/blob/main/.github/workflows/dependabot-auto-merge.yml)

For the fuller maintainer-facing explanation, see [`docs/REPOSITORY_AUTOMATION.md`](https://github.com/nicx17/mimick/blob/main/docs/REPOSITORY_AUTOMATION.md).
