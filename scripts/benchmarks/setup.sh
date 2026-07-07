#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
checkout="$repo_root/target/benchmarks/numpy"
repository="https://github.com/numpy/numpy.git"
tag="v2.4.6"
revision="b832a09cf2a169c833dd2371e7c07aa00b293242"

if [[ ! -d "$checkout/.git" ]]; then
    if [[ -e "$checkout" ]]; then
        echo "error: $checkout exists but is not a Git checkout" >&2
        exit 1
    fi
    mkdir -p "$(dirname "$checkout")"
    git clone \
        --branch "$tag" \
        --depth 1 \
        --filter=blob:none \
        "$repository" \
        "$checkout"
fi

tracked_changes="$(git -C "$checkout" status --short --untracked-files=no)"
if [[ -n "$tracked_changes" ]]; then
    echo "error: $checkout has tracked changes:" >&2
    echo "$tracked_changes" >&2
    exit 1
fi

actual="$(git -C "$checkout" rev-parse HEAD)"
if [[ "$actual" != "$revision" ]]; then
    git -C "$checkout" fetch --depth 1 origin "refs/tags/$tag"
    git -C "$checkout" checkout --detach "$revision"
    actual="$(git -C "$checkout" rev-parse HEAD)"
fi

if [[ "$actual" != "$revision" ]]; then
    echo "error: expected NumPy $revision, found $actual" >&2
    exit 1
fi
if [[ ! -d "$checkout/numpy" ]]; then
    echo "error: NumPy package directory is missing from $checkout" >&2
    exit 1
fi

untracked_python="$(git -C "$checkout" ls-files --others -- ':(glob)**/*.py')"
if [[ -n "$untracked_python" ]]; then
    echo "error: $checkout contains untracked Python sources:" >&2
    echo "$untracked_python" >&2
    exit 1
fi

file_count="$(find "$checkout/numpy" -type f -name '*.py' | wc -l | tr -d ' ')"
echo "NumPy $tag ($revision) is ready at $checkout/numpy"
echo "Discovered $file_count Python files"
