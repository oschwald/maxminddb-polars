#!/bin/bash

set -euo pipefail

dry_run=false
publish=false
case "${1:-}" in
    "") ;;
    --dry-run) dry_run=true ;;
    --publish) publish=true ;;
    *)
        echo "Usage: dev-bin/release.sh [--dry-run|--publish]" >&2
        exit 2
        ;;
esac
if (( $# > 1 )); then
    echo "Usage: dev-bin/release.sh [--dry-run|--publish]" >&2
    exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"

branch=$(git symbolic-ref --quiet --short HEAD) || {
    echo "A release cannot be prepared from a detached HEAD." >&2
    exit 1
}
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

for conversion in \
    "1.2.3-alpha.1=1.2.3a1" \
    "1.2.3-beta.2=1.2.3b2" \
    "1.2.3-rc.3=1.2.3rc3"; do
    semver=${conversion%%=*}
    expected=${conversion#*=}
    case $semver in
        *-alpha.*) actual=${semver/-alpha./a} ;;
        *-beta.*) actual=${semver/-beta./b} ;;
        *-rc.*) actual=${semver/-rc./rc} ;;
    esac
    test "$actual" = "$expected"
done

tag="v$version"

if $publish; then
    if [[ "$branch" != "main" ]]; then
        echo "--publish must be run from main." >&2
        exit 1
    fi
    if [[ -n "$(git status --porcelain)" ]]; then
        echo "The working tree must be clean." >&2
        exit 1
    fi
    if [[ "$release_date" != "$(date +%Y-%m-%d)" ]]; then
        echo "Release date $release_date is not today." >&2
        exit 1
    fi

    git fetch origin main --tags
    head=$(git rev-parse HEAD)
    if [[ "$head" != "$(git rev-parse origin/main)" ]]; then
        echo "main must exactly match origin/main." >&2
        exit 1
    fi
    cargo_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
    if [[ "$cargo_version" != "$version" ]]; then
        echo "Cargo.toml version $cargo_version does not match CHANGELOG.md $version." >&2
        exit 1
    fi
    if git rev-parse --verify --quiet "$tag" >/dev/null || \
        git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1 || \
        gh release view "$tag" >/dev/null 2>&1; then
        echo "Tag or release $tag already exists." >&2
        exit 1
    fi

    cargo publish --dry-run --locked
    project_status=$(curl --silent --show-error --location --output /dev/null \
        --write-out '%{http_code}' \
        --user-agent "maxminddb-polars-release/$version (https://github.com/oschwald/maxminddb-polars)" \
        https://crates.io/api/v1/crates/maxminddb-polars)
    if [[ "$project_status" == "404" ]]; then
        echo "The first crates.io version requires the local Cargo API token."
        read -r -p "Bootstrap maxminddb-polars $version on crates.io? [y/N] " answer
        if [[ "$answer" != "y" ]]; then
            echo "Aborting before any release was created."
            exit 1
        fi
        cargo publish --locked
        echo "Configure the crate's trusted publisher for release.yml and the release environment."
    elif [[ "$project_status" != "200" ]]; then
        echo "Could not check crates.io (HTTP $project_status)." >&2
        exit 1
    fi

    echo
    echo "Release notes:"
    printf '%s\n' "$notes"
    echo
    read -r -p "Create GitHub release $tag and start publication? [y/N] " answer
    if [[ "$answer" != "y" ]]; then
        echo "Stopping before the GitHub release was created."
        exit 1
    fi
    gh release create --target "$head" --title "$version" --notes "$notes" "$tag"
    echo "Release created. Follow the Release workflow until publication completes."
    exit 0
fi

if $dry_run; then
    if [[ -n "$(git status --porcelain)" ]]; then
        echo "The working tree must be clean for a dry run." >&2
        exit 1
    fi
else
    if ! git diff --cached --quiet || [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
        echo "The index and untracked-file set must be clean." >&2
        exit 1
    fi
    while IFS= read -r changed_file; do
        if [[ -n "$changed_file" && "$changed_file" != "CHANGELOG.md" ]]; then
            echo "Only CHANGELOG.md may be edited before preparing a release." >&2
            exit 1
        fi
    done < <(git diff --name-only)
fi

if ! $dry_run; then
    if [[ "$branch" == "main" ]]; then
        echo "Create release/$tag from origin/main before preparing a release." >&2
        exit 1
    fi
    if [[ "$branch" != "release/$tag" ]]; then
        echo "Expected branch release/$tag; found $branch." >&2
        exit 1
    fi
    if [[ "$release_date" != "$(date +%Y-%m-%d)" ]]; then
        echo "Release date $release_date is not today." >&2
        exit 1
    fi
    git fetch origin main
    if ! git merge-base --is-ancestor origin/main HEAD; then
        echo "The release branch is behind origin/main." >&2
        exit 1
    fi
    if git rev-parse --verify --quiet "$tag" >/dev/null || \
        git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
        echo "Tag $tag already exists." >&2
        exit 1
    fi

    perl -0pi -e \
        's/^version = "[^"]+"/version = "'"$version"'"/m' Cargo.toml
    cargo check
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
git diff -- Cargo.toml Cargo.lock README.md CHANGELOG.md
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

read -r -p "Create the release-preparation commit and push this branch? [y/N] " answer
if [[ "$answer" != "y" ]]; then
    echo "Leaving the validated release changes in the working tree."
    exit 1
fi

git add Cargo.toml Cargo.lock README.md CHANGELOG.md
git commit -m "Prepare $tag release"
git push --set-upstream origin "$branch"

echo "Release preparation pushed. Open a pull request; this script did not tag or publish."
