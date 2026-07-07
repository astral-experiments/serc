#!/usr/bin/env bash

set -euo pipefail

checkout="$1"
corpus="$2"

tracked_changes="$(git -C "$checkout" status --short --untracked-files=no -- "$corpus")"
untracked_paths="$(git -C "$checkout" ls-files --others --directory -- "$corpus")"

if [[ -n "$tracked_changes" || -n "$untracked_paths" ]]; then
    echo "error: the pinned corpus contains filesystem changes:" >&2
    if [[ -n "$tracked_changes" ]]; then
        echo "$tracked_changes" >&2
    fi
    if [[ -n "$untracked_paths" ]]; then
        echo "$untracked_paths" >&2
    fi
    exit 1
fi
