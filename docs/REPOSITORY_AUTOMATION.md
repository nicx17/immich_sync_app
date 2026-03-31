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

Dependabot configuration lives in [`.github/dependabot.yml`](../.github/dependabot.yml).

It currently updates:

- Cargo dependencies
- GitHub Actions workflow actions

Expected repository labels:

- `dependencies`
- `rust`
- `github-actions`

## CODEOWNERS

Ownership rules live in [`.github/CODEOWNERS`](../.github/CODEOWNERS).

The default owner is:

- `@nicx17`

This is most useful when combined with branch protection or rulesets that require code-owner review.

## Branch Protection On `main`

For a solo-maintainer setup, `main` should be protected in GitHub with these native rules:

- required status checks:
  - `Format, Lint, and Test`
  - `Dependency Audit`
  - `Verify cargo-sources.json`
- stale approvals dismissed on new commits
- required conversation resolution
- admins are not enforced

Do not rely on GitHub's native `required approving review` rule for a solo-maintainer repository. GitHub does not let the PR author satisfy that rule by approving their own pull request, so enabling it will block your own PRs from ever becoming "approved" in the normal way.

Instead, this repo uses the `Maintainer Approval Gate` status check to require `@nicx17` approval on pull requests opened by other people, while allowing PRs authored by `@nicx17` to pass without a second account.

## Maintainer Approval Gate

Approval enforcement for pull requests opened by other people is handled by [`.github/workflows/maintainer-approval.yml`](../.github/workflows/maintainer-approval.yml).

Behavior:

- PRs opened by `@nicx17` pass automatically
- PRs opened by anyone else require the latest review from `@nicx17` to be `APPROVED`

This avoids forcing you to find a second reviewer for your own pull requests while still preventing other contributors from merging without your sign-off.

This workflow is now a defense-in-depth signal. The authoritative merge policy on `main` is the native branch protection described above.

## Cargo Vendor Guard

The Flatpak manifests depend on [cargo-sources.json](../cargo-sources.json), so dependency updates are not complete unless the vendored source list stays in sync.

The guard workflow is [`.github/workflows/cargo-sources-guard.yml`](../.github/workflows/cargo-sources-guard.yml).

It runs on every pull request to `main`, and it fails when:

- `Cargo.lock` changes
- `cargo-sources.json` does not change

Expected fix:

```bash
uv run flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
```

The workflow allows a root package version-only `Cargo.lock` change for release prep.

## Docs Link Check

Documentation link checks run from [`.github/workflows/docs-links.yml`](../.github/workflows/docs-links.yml).

It validates links in:

- `README.md`
- `docs/**/*.md`
- `wiki/**/*.md`

The workflow excludes `img.shields.io` badge URLs because those are noisy and not useful as a release blocker.

## Release Drafter

Release Drafter is configured by:

- [`.github/release-drafter.yml`](../.github/release-drafter.yml)
- [`.github/workflows/release-drafter.yml`](../.github/workflows/release-drafter.yml)

It helps maintain a draft GitHub release based on merged PR labels and categories.

The workflow now serializes draft updates and avoids running a redundant PR-close pass, which helps prevent duplicate draft releases during merges.

Tagged releases now prefer the Release Drafter draft whose tag or title matches the version being published. If no matching draft release exists, the release workflow falls back to the matching section in [CHANGELOG.md](../CHANGELOG.md).

## Dependabot Auto Merge

Approved dependency PRs can be armed for native GitHub auto-merge by [`.github/workflows/dependabot-auto-merge.yml`](../.github/workflows/dependabot-auto-merge.yml).

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
2. `main` is protected with required CI checks and conversation resolution.
3. `Maintainer Approval Gate` is included in the required status checks if you want outside-contributor PRs to require `@nicx17` approval.

Optional extras you may still want later:

1. re-enable native required approving reviews if you add another maintainer who can review your PRs
2. enforce admin rules too, if you ever want your own PRs to need the same bypass restrictions
3. add more required checks if additional always-on workflows become important enough to gate merges

## Practical Notes

- Public repositories can use GitHub-hosted Actions without the same billing pressure as private repos.
- Dependabot dependency PRs still need human judgment for larger dependency jumps.
- Release Drafter is now the preferred GitHub release-notes source when a draft exists, while [CHANGELOG.md](../CHANGELOG.md) remains the fallback and long-form project history.
- GitHub Actions cannot override the native "required approving review" rule for self-authored PRs; if you enable that rule, plan on a second reviewer or manual admin bypass.
