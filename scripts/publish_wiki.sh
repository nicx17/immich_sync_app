#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wiki_source="$repo_root/wiki"

if [[ ! -d "$wiki_source" ]]; then
  echo "Wiki source directory not found: $wiki_source" >&2
  exit 1
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/mimick-wiki.XXXXXX")"
trap 'rm -rf "$tmpdir"' EXIT

origin_url="$(git -C "$repo_root" remote get-url origin)"
repo_path=""
default_wiki_url=""

case "$origin_url" in
  git@github.com:*.git)
    repo_path="${origin_url#git@github.com:}"
    default_wiki_url="git@github.com:${repo_path%.git}.wiki.git"
    ;;
  https://github.com/*.git)
    repo_path="${origin_url#https://github.com/}"
    default_wiki_url="https://github.com/${repo_path%.git}.wiki.git"
    ;;
  *)
    echo "Unsupported origin URL format: $origin_url" >&2
    exit 1
    ;;
esac

repo_path="${repo_path%.git}"
wiki_url="${WIKI_REMOTE_URL:-$default_wiki_url}"

if ! git clone "$wiki_url" "$tmpdir"; then
  echo "Failed to clone $wiki_url." >&2
  echo "Make sure the GitHub wiki is enabled for this repository and that your git credentials can access it." >&2
  echo "You can also override the remote with WIKI_REMOTE_URL=..." >&2
  exit 1
fi

find "$tmpdir" -maxdepth 1 -type f -name '*.md' -delete
cp "$wiki_source"/*.md "$tmpdir"/

cd "$tmpdir"
git add .

if git diff --cached --quiet; then
  echo "Wiki already up to date."
  exit 0
fi

git commit -m "Refresh project wiki"
if ! git push origin master; then
  echo "Failed to push the wiki changes to GitHub." >&2
  echo "If you use HTTPS remotes, make sure a git credential helper or token-based auth is configured." >&2
  echo "You can also override the wiki remote with WIKI_REMOTE_URL=..." >&2
  exit 1
fi
