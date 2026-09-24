#!/usr/bin/env bash
# Prepare a release commit with Rooster, as in uv.
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ -n "$(git status --porcelain)" ]]; then
    echo "Release preparation requires a clean checkout" >&2
    exit 1
fi

# The first release has no prior tag; use the version already in the manifests.
if [[ $# == 0 ]] && ! git tag --list | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    set -- --version "$(uv version --short)"
fi
uv run --locked --default-index https://pypi.org/simple --python 3.12 --only-group release rooster release --no-update-version-files "$@"
version="$(python3 scripts/update-release-version.py)"
python3 scripts/check-release.py
cargo update -p serc -p serc-cli
uv lock --default-index https://pypi.org/simple

git switch -c "charlie/release-$version"
git add CHANGELOG.md pyproject.toml uv.lock Cargo.lock crates/serc/Cargo.toml crates/serc_cli/Cargo.toml
git commit -m "Bump version to $version"
