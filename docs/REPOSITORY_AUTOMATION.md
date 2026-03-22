# Repository Automation

This document describes the automation stack used by the Mimick repository.

## Overview

The repo currently uses GitHub-native automation plus a small number of focused workflows:

- `Dependabot` for Cargo and GitHub Actions dependency updates
- `CODEOWNERS` for ownership and review guidance
- `Maintainer Approval Gate` to require `@nicx17` approval on PRs opened by other people
- `Cargo Vendor Guard` to catch missing `cargo-sources.json` refreshes
- `Docs Link Check` to validate README, docs, and wiki links
- `Release Drafter` to keep a draft release page up to date
- `Dependabot Auto Merge` to enable native auto-merge for approved dependency PRs

This setup aims to improve release safety and repo hygiene without turning the project into bot soup.

## Dependabot

Dependabot configuration lives in [`.github/dependabot.yml`](/home/nick/Documents/Github/immich_sync_app/.github/dependabot.yml).

It currently updates:

- Cargo dependencies
- GitHub Actions workflow actions

Expected repository labels:

- `dependencies`
- `rust`
- `github-actions`

## CODEOWNERS

Ownership rules live in [`.github/CODEOWNERS`](/home/nick/Documents/Github/immich_sync_app/.github/CODEOWNERS).

The default owner is:

- `@nicx17`

This is most useful when combined with branch protection or rulesets that require code-owner review.

## Branch Protection On `main`

`main` is now protected in GitHub with these native rules:

- required status checks:
  - `Format, Lint, and Test`
  - `Dependency Audit`
  - `Verify cargo-sources.json`
- 1 required approving review
- required code-owner review
- stale approvals dismissed on new commits
- required conversation resolution
- admins are not enforced

Because [`.github/CODEOWNERS`](/home/nick/Documents/Github/immich_sync_app/.github/CODEOWNERS) assigns the repo to `@nicx17`, pull requests opened by other people need your approval before merging.

Your own pull requests are still practical to merge because admin enforcement is left off.

## Maintainer Approval Gate

Approval enforcement for pull requests opened by other people is handled by [`.github/workflows/maintainer-approval.yml`](/home/nick/Documents/Github/immich_sync_app/.github/workflows/maintainer-approval.yml).

Behavior:

- PRs opened by `@nicx17` pass automatically
- PRs opened by anyone else require the latest review from `@nicx17` to be `APPROVED`

This avoids forcing you to find a second reviewer for your own pull requests while still preventing other contributors from merging without your sign-off.

This workflow is now a defense-in-depth signal. The authoritative merge policy on `main` is the native branch protection described above.

## Cargo Vendor Guard

The Flatpak manifests depend on [cargo-sources.json](/home/nick/Documents/Github/immich_sync_app/cargo-sources.json), so dependency updates are not complete unless the vendored source list stays in sync.

The guard workflow is [`.github/workflows/cargo-sources-guard.yml`](/home/nick/Documents/Github/immich_sync_app/.github/workflows/cargo-sources-guard.yml).

It runs on every pull request to `main`, and it fails when:

- `Cargo.lock` changes
- `cargo-sources.json` does not change

Expected fix:

```bash
uv run flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
```

The workflow allows a root package version-only `Cargo.lock` change for release prep.

## Docs Link Check

Documentation link checks run from [`.github/workflows/docs-links.yml`](/home/nick/Documents/Github/immich_sync_app/.github/workflows/docs-links.yml).

It validates links in:

- `README.md`
- `docs/**/*.md`
- `wiki/**/*.md`

The workflow excludes `img.shields.io` badge URLs because those are noisy and not useful as a release blocker.

## Release Drafter

Release Drafter is configured by:

- [`.github/release-drafter.yml`](/home/nick/Documents/Github/immich_sync_app/.github/release-drafter.yml)
- [`.github/workflows/release-drafter.yml`](/home/nick/Documents/Github/immich_sync_app/.github/workflows/release-drafter.yml)

It helps maintain a draft GitHub release based on merged PR labels and categories.

This does not replace the manual changelog. Mimick still uses [CHANGELOG.md](/home/nick/Documents/Github/immich_sync_app/CHANGELOG.md) as the canonical release notes source for tagged releases.

## Dependabot Auto Merge

Approved dependency PRs can be armed for native GitHub auto-merge by [`.github/workflows/dependabot-auto-merge.yml`](/home/nick/Documents/Github/immich_sync_app/.github/workflows/dependabot-auto-merge.yml).

The workflow only targets pull requests that are:

- opened by `dependabot[bot]`
- labeled `dependencies`
- approved by `@nicx17`
- not drafts

Important:

- this workflow only enables GitHub's native auto-merge
- the repository must have **Allow auto-merge** enabled in GitHub settings
- branch protection or rulesets should require approval if you want "approved dependency PRs" to mean something enforceable

## Recommended GitHub Settings

These GitHub settings are already the intended baseline for this repo:

1. **Allow auto-merge** is enabled.
2. `main` is protected with required CI checks and review rules.
3. Code-owner review is required for outside pull requests.

Optional extras you may still want later:

1. enforce admin rules too, if you ever want your own PRs to need the same review path
2. add more required checks if additional always-on workflows become important enough to gate merges

## Practical Notes

- Public repositories can use GitHub-hosted Actions without the same billing pressure as private repos.
- Dependabot dependency PRs still need human judgment for larger dependency jumps.
- Release Drafter is a convenience layer, not the source of truth for Mimick release notes.
