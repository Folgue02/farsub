#!/usr/bin/env bash
# Set a new version (semver) and tag it.
#
#   scripts/bump-version.sh patch     # 0.1.0 -> 0.1.1  (fixes only)
#   scripts/bump-version.sh minor     # 0.1.0 -> 0.2.0  (new features)
#   scripts/bump-version.sh major     # 0.1.0 -> 1.0.0  (breaking changes)
#   scripts/bump-version.sh 0.3.0     # an exact version
#   scripts/bump-version.sh --tag     # tag the current version as it is (first release)
#
# Updates Cargo.toml and Cargo.lock, commits "Release vX.Y.Z" and creates the annotated
# tag vX.Y.Z. Nothing is pushed: `git push --follow-tags` sends the commit and the tag.
set -euo pipefail
cd "$(dirname "$0")/.."

[ $# -eq 1 ] || { sed -n '2,12p' "$0"; exit 2; }
if [ -n "$(git status --porcelain)" ]; then
    echo "Uncommitted changes: commit them first." >&2
    exit 1
fi

cur=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
IFS=. read -r major minor patch <<<"${cur%%-*}"
case "$1" in
    --tag) new=$cur ;;
    major) new="$((major + 1)).0.0" ;;
    minor) new="$major.$((minor + 1)).0" ;;
    patch) new="$major.$minor.$((patch + 1))" ;;
    *)
        [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || {
            echo "not a semver version: $1" >&2
            exit 2
        }
        new=$1
        ;;
esac

if git rev-parse -q --verify "refs/tags/v$new" >/dev/null; then
    echo "tag v$new already exists" >&2
    exit 1
fi

if [ "$new" != "$cur" ]; then
    # Only the package's own version line (the first one) changes.
    awk -v v="$new" '!done && /^version = "/ { print "version = \"" v "\""; done=1; next } { print }' \
        Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
    cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null
    git add Cargo.toml Cargo.lock
    git commit -q -m "Release v$new"
fi
git tag -a "v$new" -m "farsub $new"
echo "Tagged v$new. Push with: git push --follow-tags"
