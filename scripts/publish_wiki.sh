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

case "$origin_url" in
  git@github.com:*.git)
    wiki_url="${origin_url%.git}.wiki.git"
    ;;
  https://github.com/*.git)
    wiki_url="${origin_url%.git}.wiki.git"
    ;;
  *)
    echo "Unsupported origin URL format: $origin_url" >&2
    exit 1
    ;;
esac

if ! git clone "$wiki_url" "$tmpdir"; then
  echo "Failed to clone $wiki_url." >&2
  echo "Make sure the GitHub wiki is enabled for this repository and that your git credentials can access it." >&2
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
git push origin master
