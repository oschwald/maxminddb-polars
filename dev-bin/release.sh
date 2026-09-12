#!/bin/bash

set -euo pipefail

dry_run=false
case "${1:-}" in
    "") ;;
    --dry-run) dry_run=true ;;
    *)
        echo "Usage: dev-bin/release.sh [--dry-run]" >&2
        exit 2
        ;;
esac
if (( $# > 1 )); then
    echo "Usage: dev-bin/release.sh [--dry-run]" >&2
    exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"

branch=$(git symbolic-ref --quiet --short HEAD) || {
    echo "A release cannot be run from a detached HEAD." >&2
    exit 1
}
if [[ -n "$(git status --porcelain)" ]]; then
    echo "The working tree must be clean. Commit the changelog before releasing." >&2
    exit 1
fi

release_pattern='^## \[([0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?)\] - ([0-9]{4}-[0-9]{2}-[0-9]{2})$'
release_heading=$(grep -m1 -E "$release_pattern" CHANGELOG.md || true)

if [[ -n "$release_heading" ]]; then
    [[ "$release_heading" =~ $release_pattern ]]
    version=${BASH_REMATCH[1]}
    release_date=${BASH_REMATCH[4]}
    notes=$(awk -v heading="$release_heading" '
        $0 == heading { found = 1; next }
        found && /^## / { exit }
        found { print }
    ' CHANGELOG.md)
else
    if ! $dry_run; then
        echo "CHANGELOG.md has no dated release entry." >&2
        exit 1
    fi
    version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
    release_date=$(date +%Y-%m-%d)
    notes="Dry run for the unreleased package skeleton."
fi

if [[ "$version" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-(alpha|beta|rc)\.([0-9]+)$ ]]; then
    case ${BASH_REMATCH[2]} in
        alpha) prerelease=a ;;
        beta) prerelease=b ;;
        rc) prerelease=rc ;;
    esac
    pep440_version="${BASH_REMATCH[1]}${prerelease}${BASH_REMATCH[3]}"
elif [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    pep440_version=$version
else
    echo "Unsupported release version: $version" >&2
    exit 1
fi

tag="v$version"

if ! $dry_run; then
    if [[ "$branch" == "main" ]]; then
        echo "Create a release branch from origin/main before releasing." >&2
        exit 1
    fi
    if [[ "$release_date" != "$(date +%Y-%m-%d)" ]]; then
        echo "Release date $release_date is not today." >&2
        exit 1
    fi
    git fetch origin main --tags
    if ! git merge-base --is-ancestor origin/main HEAD; then
        echo "The release branch is behind origin/main." >&2
        exit 1
    fi
    if git rev-parse --verify --quiet "$tag" >/dev/null || \
        gh release view "$tag" >/dev/null 2>&1; then
        echo "Tag or release $tag already exists." >&2
        exit 1
    fi

    # The tree started clean; discard generated version changes if validation
    # fails or the release is declined. Keep them once they have been committed.
    trap 'git restore --source=HEAD --staged --worktree -- Cargo.toml Cargo.lock fuzz/Cargo.lock' EXIT

    perl -0pi -e \
        's/^version = "[^"]+"/version = "'"$version"'"/m' Cargo.toml
    cargo check
    cargo check --manifest-path fuzz/Cargo.toml
fi

scripts/check
cargo publish --dry-run --locked --allow-dirty

artifact_dir=$(mktemp -d)
uv run --no-sync maturin build --release --locked --out "$artifact_dir"
uv run --no-sync maturin sdist --out "$artifact_dir"
uv run --no-sync python scripts/inspect_artifacts.py \
    --expected-version "$pep440_version" "$artifact_dir"/*
uv run --no-sync twine check --strict "$artifact_dir"/*

echo
echo "Release diff:"
git diff -- Cargo.toml Cargo.lock fuzz/Cargo.lock
echo
echo "Release notes:"
printf '%s\n' "$notes"
echo
echo "Validated $tag (PyPI version $pep440_version)."
echo "Artifacts are in $artifact_dir."

if $dry_run; then
    echo "Dry run complete; nothing was committed, pushed, tagged, or published."
    exit 0
fi

read -r -p "Commit changes, push to origin, and create GitHub release $tag? [y/N] " answer
if [[ ! "$answer" =~ ^[Yy]([Ee][Ss])?$ ]]; then
    echo "Aborting."
    exit 1
fi

git add Cargo.toml Cargo.lock fuzz/Cargo.lock
if ! git diff --cached --quiet; then
    git commit -m "Prepare $tag release"
fi
trap - EXIT

head=$(git rev-parse HEAD)
git push --set-upstream origin "$branch"
gh release create --target "$head" --title "$version" --notes "$notes" "$tag"

echo "Release created. Follow the Release workflow until publication completes."
echo "Open a pull request to merge $branch into main."
