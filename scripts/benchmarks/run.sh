#!/usr/bin/env bash

set -euo pipefail

usage() {
    cat <<'EOF'
usage: scripts/benchmarks/run.sh [CORPUS [RESULTS_JSON]]

Environment:
  PYTHON                 CPython executable (default: python3.14)
  HYPERFINE_WARMUP       Warmup runs (default: 3)
  HYPERFINE_MIN_RUNS     Minimum measured runs (default: 10)
EOF
}

if [[ $# -gt 2 ]]; then
    usage >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
numpy_checkout="$repo_root/target/benchmarks/numpy"
numpy_revision="b832a09cf2a169c833dd2371e7c07aa00b293242"
default_corpus="$numpy_checkout/numpy"

python="${PYTHON:-python3.14}"
corpus="${1:-$default_corpus}"
results_json="${2:-$repo_root/target/benchmarks/results/numpy.json}"
warmup="${HYPERFINE_WARMUP:-3}"
min_runs="${HYPERFINE_MIN_RUNS:-10}"
serc="$repo_root/target/release/serc"

if ! command -v "$python" >/dev/null 2>&1; then
    echo "error: CPython executable not found: $python" >&2
    exit 1
fi
if ! command -v hyperfine >/dev/null 2>&1; then
    echo "error: hyperfine is required; see https://github.com/sharkdp/hyperfine#installation" >&2
    exit 1
fi
if [[ ! -d "$corpus" ]]; then
    echo "error: corpus directory not found: $corpus" >&2
    echo "hint: run ./scripts/benchmarks/setup.sh" >&2
    exit 1
fi

corpus="$(cd "$corpus" && pwd -P)"
using_pinned_numpy=false
if [[ -d "$default_corpus" ]]; then
    default_corpus="$(cd "$default_corpus" && pwd -P)"
    if [[ "$corpus" == "$default_corpus" ]]; then
        using_pinned_numpy=true
    fi
fi

if [[ "$using_pinned_numpy" == true ]]; then
    actual_revision="$(git -C "$numpy_checkout" rev-parse HEAD)"
    tracked_changes="$(git -C "$numpy_checkout" status --short --untracked-files=no)"
    untracked_python="$(git -C "$numpy_checkout" ls-files --others -- ':(glob)**/*.py')"
    if [[ "$actual_revision" != "$numpy_revision" ]]; then
        echo "error: expected NumPy $numpy_revision, found $actual_revision" >&2
        exit 1
    fi
    if [[ -n "$tracked_changes" || -n "$untracked_python" ]]; then
        echo "error: the pinned NumPy corpus contains source changes; rerun setup or clean it" >&2
        exit 1
    fi
fi

python="$(command -v "$python")"
mkdir -p "$(dirname "$results_json")"
results_json="$(cd "$(dirname "$results_json")" && pwd -P)/$(basename "$results_json")"
if [[ "$results_json" == *.json ]]; then
    results_markdown="${results_json%.json}.md"
    results_metadata="${results_json%.json}.metadata.txt"
else
    results_markdown="$results_json.md"
    results_metadata="$results_json.metadata.txt"
fi

"$python" -I -c '
import importlib.util
import sys

expected = ("cpython", (3, 14, 5), "2b0e0d0a")
actual = (
    sys.implementation.name,
    sys.version_info[:3],
    importlib.util.MAGIC_NUMBER.hex(),
)
if actual != expected:
    raise SystemExit(f"expected {expected}, got {actual}")
'

echo "Building Serc in release mode"
CARGO_INCREMENTAL=0 cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --release \
    --locked \
    --package serc \
    --bin serc

clean_caches() {
    find "$corpus" -type d -name __pycache__ -prune -exec rm -rf {} +
}

source_count="$(find "$corpus" -type f -name '*.py' | wc -l | tr -d ' ')"
if [[ "$using_pinned_numpy" == true && "$source_count" != 505 ]]; then
    echo "error: expected 505 NumPy Python files, found $source_count" >&2
    exit 1
fi
source_bytes="$("$python" -I -c '
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
print(sum(path.stat().st_size for path in root.rglob("*.py") if path.is_file()))
' "$corpus")"
echo "Preflighting CPython and Serc on $source_count files ($source_bytes bytes) in $corpus"

clean_caches
"$python" -I -m compileall \
    -q \
    -f \
    -j 1 \
    -o 0 \
    --invalidation-mode timestamp \
    "$corpus"
cpython_count="$(find "$corpus" -type f -path '*/__pycache__/*.pyc' | wc -l | tr -d ' ')"

clean_caches
"$serc" compile --quiet "$corpus"
serc_count="$(find "$corpus" -type f -path '*/__pycache__/*.pyc' | wc -l | tr -d ' ')"

if [[ "$cpython_count" != "$source_count" || "$serc_count" != "$source_count" ]]; then
    echo "error: expected $source_count outputs; CPython wrote $cpython_count and Serc wrote $serc_count" >&2
    exit 1
fi

echo "Preflight checks successful compilation and output counts, not byte identity"
clean_caches

printf -v cpython_command \
    '%q -I -m compileall -q -f -j 1 -o 0 --invalidation-mode timestamp %q' \
    "$python" \
    "$corpus"
printf -v serc_command '%q compile --quiet %q' "$serc" "$corpus"
printf -v prepare_command \
    'find %q -type d -name __pycache__ -prune -exec rm -rf {} +' \
    "$corpus"

serc_revision="$(git -C "$repo_root" rev-parse --short HEAD)"
if [[ -n "$(git -C "$repo_root" status --short)" ]]; then
    serc_revision="$serc_revision+dirty"
fi
corpus_revision="$(git -C "$corpus" rev-parse HEAD 2>/dev/null || echo unavailable)"
{
    echo "timestamp_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "corpus=$corpus"
    echo "corpus_revision=$corpus_revision"
    echo "source_files=$source_count"
    echo "source_bytes=$source_bytes"
    echo "serc_revision=$serc_revision"
    echo "python_executable=$python"
    echo
    echo "[python]"
    "$python" -VV
    echo
    echo "[rustc]"
    rustc --version --verbose
    echo
    echo "[cargo]"
    cargo --version --verbose
    echo
    echo "[hyperfine]"
    hyperfine --version
    echo
    echo "[platform]"
    uname -a
    if command -v lscpu >/dev/null 2>&1; then
        lscpu
    fi
    echo
    echo "[serc status]"
    git -C "$repo_root" status --short
    echo
    echo "[corpus status]"
    git -C "$corpus" status --short 2>/dev/null || true
} > "$results_metadata"
echo "Running Hyperfine; results will be written to:"
echo "  $results_json"
echo "  $results_markdown"
echo "  $results_metadata"

hyperfine \
    --warmup "$warmup" \
    --min-runs "$min_runs" \
    --export-json "$results_json" \
    --export-markdown "$results_markdown" \
    --command-name "CPython 3.14.5 compileall" \
    --command-name "Serc $serc_revision" \
    --prepare "$prepare_command" \
    --prepare "$prepare_command" \
    "$cpython_command" \
    "$serc_command"
