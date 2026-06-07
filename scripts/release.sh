#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "Usage: $0 <major|minor|patch|x.y.z>"
    exit 1
}

[[ $# -ne 1 ]] && usage

CARGO_TOML="$(dirname "$0")/../Cargo.toml"

current=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/')
IFS='.' read -r major minor patch <<< "$current"

case "$1" in
    major) major=$((major + 1)); minor=0; patch=0 ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    patch) patch=$((patch + 1)) ;;
    [0-9]*.[0-9]*.[0-9]*) major=${1%%.*}; rest=${1#*.}; minor=${rest%%.*}; patch=${rest##*.} ;;
    *) usage ;;
esac

next="$major.$minor.$patch"
echo "  $current → $next"

sed -i '' "s/^version = \"$current\"/version = \"$next\"/" "$CARGO_TOML"

git add .
git commit -m "chore: bump version to $next"
git tag "v$next"

echo "Tagged v$next — push with: git push && git push origin v$next"
