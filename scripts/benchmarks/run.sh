#!/usr/bin/env bash

set -euo pipefail

usage() {
    cat <<'EOF'
usage: scripts/benchmarks/run.sh [CORPUS [RESULTS_JSON]]

Environment:
  PYTHON                 CPython executable (default: python3.14)
  HYPERFINE_WARMUP       Warmup runs (default: 3)
  HYPERFINE_MIN_RUNS     Minimum measured runs (default: 10)
  SERC_PGO_PROFILE    Optional merged profile from scripts/build-pgo-release
EOF
}

if [[ $# -gt 2 ]]; then
    usage >&2
    exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/../.." && pwd -P)"
numpy_checkout="$repo_root/target/benchmarks/numpy"
numpy_revision="b832a09cf2a169c833dd2371e7c07aa00b293242"
default_corpus="$numpy_checkout/numpy"
serc_profile="release"
serc_build_mode="release"
pgo_profile="${SERC_PGO_PROFILE:-}"
pgo_profile_sha256=""
pgo_target=""
build_target_args=()

python="${PYTHON:-python3.14}"
corpus="${1:-$default_corpus}"
results_json="${2:-$repo_root/target/benchmarks/results/numpy.json}"
warmup="${HYPERFINE_WARMUP:-3}"
min_runs="${HYPERFINE_MIN_RUNS:-10}"

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

clean_caches() {
    find "$corpus" -type d -name __pycache__ -prune -exec rm -rf {} +
}

tree_summary() {
    "$python" -I -S -c '
import pathlib
import sys

sys.path.insert(0, sys.argv[1])
from pgo_corpus import tree_summary

summary = tree_summary(pathlib.Path(sys.argv[2]), sys.argv[3])
print(summary["files"], summary["bytes"], summary["sha256"], sep="\t")
' "$repo_root/scripts" "$1" "$2"
}

sha256_file() {
    "$python" -I -S -c '
import hashlib
import pathlib
import sys

print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
' "$1"
}

sha256_stdin() {
    "$python" -I -S -c 'import hashlib, sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())'
}

if [[ "$using_pinned_numpy" == true ]]; then
    clean_caches
    actual_revision="$(git -C "$numpy_checkout" rev-parse HEAD)"
    if [[ "$actual_revision" != "$numpy_revision" ]]; then
        echo "error: expected NumPy $numpy_revision, found $actual_revision" >&2
        exit 1
    fi
    if ! bash "$repo_root/scripts/benchmarks/validate-corpus.sh" "$numpy_checkout" numpy; then
        echo "hint: rerun setup or clean the pinned NumPy corpus" >&2
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

"$python" -I -S -c '
import importlib.util
import sys

expected = ("cpython", (3, 14, 5, "final", 0), "2b0e0d0a")
actual = (
    sys.implementation.name,
    tuple(sys.version_info),
    importlib.util.MAGIC_NUMBER.hex(),
)
if actual != expected:
    raise SystemExit(f"expected {expected}, got {actual}")
'

if [[ -n "$pgo_profile" ]]; then
    if [[ -n "${RUSTFLAGS:-}" || -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]]; then
        echo "error: unset RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS when using SERC_PGO_PROFILE" >&2
        exit 1
    fi
    if [[ ! -f "$pgo_profile" ]]; then
        echo "error: PGO profile not found: $pgo_profile" >&2
        exit 1
    fi
    pgo_profile="$(cd "$(dirname "$pgo_profile")" && pwd -P)/$(basename "$pgo_profile")"
    pgo_profile_sha256="$(sha256_file "$pgo_profile")"
    export CARGO_ENCODED_RUSTFLAGS="-Cprofile-use=$pgo_profile"
    export CARGO_PROFILE_RELEASE_PANIC=unwind
    pgo_target="$(rustc --version --verbose | sed -n 's/^host: //p')"
    build_target_args=(--target "$pgo_target")
    serc_build_mode="release-pgo"
fi

echo "Building Serc with the $serc_build_mode profile"
serc="$(
    CARGO_INCREMENTAL=0 cargo build \
        --manifest-path "$repo_root/Cargo.toml" \
        --profile "$serc_profile" \
        "${build_target_args[@]}" \
        --locked \
        --package serc-cli \
        --bin serc \
        --message-format json-render-diagnostics \
        | bash "$repo_root/scripts/benchmarks/resolve-cargo-artifact.sh" "$python"
)"
if [[ ! -x "$serc" ]]; then
    echo "error: Cargo reported a Serc executable that is missing or not executable: $serc" >&2
    exit 1
fi
serc="$(cd "$(dirname "$serc")" && pwd -P)/$(basename "$serc")"

source_summary="$(tree_summary "$corpus" .py)"
read -r source_count source_bytes corpus_sha256 <<< "$source_summary"
if [[ ! "$source_count" =~ ^[0-9]+$ || ! "$source_bytes" =~ ^[0-9]+$ || ${#corpus_sha256} -ne 64 ]]; then
    echo "error: failed to summarize the benchmark corpus: $source_summary" >&2
    exit 1
fi
if [[ "$source_count" -eq 0 ]]; then
    echo "error: benchmark corpus contains no Python sources: $corpus" >&2
    exit 1
fi
if [[ "$using_pinned_numpy" == true && "$source_count" != 505 ]]; then
    echo "error: expected 505 NumPy Python files, found $source_count" >&2
    exit 1
fi
echo "Preflighting CPython and Serc on $source_count files ($source_bytes bytes) in $corpus"

clean_caches
"$python" -I -S -m compileall \
    -q \
    -f \
    -j 1 \
    -o 0 \
    --invalidation-mode timestamp \
    "$corpus"
cpython_summary="$(tree_summary "$corpus" .pyc)"
read -r cpython_count _cpython_bytes cpython_pyc_sha256 <<< "$cpython_summary"

clean_caches
"$serc" compile --quiet --jobs 1 "$corpus"
serc_summary="$(tree_summary "$corpus" .pyc)"
read -r serc_count _serc_bytes serc_pyc_sha256 <<< "$serc_summary"

if [[ "$cpython_count" != "$source_count" || "$serc_count" != "$source_count" ]]; then
    echo "error: expected $source_count outputs; CPython wrote $cpython_count and Serc wrote $serc_count" >&2
    exit 1
fi
if [[ ${#cpython_pyc_sha256} -ne 64 || ${#serc_pyc_sha256} -ne 64 ]]; then
    echo "error: failed to hash preflight bytecode outputs" >&2
    exit 1
fi
if [[ "$cpython_pyc_sha256" != "$serc_pyc_sha256" ]]; then
    echo "error: preflight bytecode differs from CPython" >&2
    echo "CPython aggregate: $cpython_pyc_sha256" >&2
    echo "Serc aggregate:  $serc_pyc_sha256" >&2
    exit 1
fi

echo "Preflight verified byte-identical outputs ($cpython_pyc_sha256)"
clean_caches

printf -v cpython_command \
    '%q -I -S -m compileall -q -f -j 1 -o 0 --invalidation-mode timestamp %q' \
    "$python" \
    "$corpus"
printf -v serc_command '%q compile --quiet --jobs 1 %q' "$serc" "$corpus"
printf -v prepare_command \
    'find %q -type d -name __pycache__ -prune -exec rm -rf {} +' \
    "$corpus"

serc_commit="$(git -C "$repo_root" rev-parse 'HEAD^{commit}')"
serc_status="$(git -C "$repo_root" status --short)"
serc_dirty_diff_sha256="$(git -C "$repo_root" diff --binary --full-index --no-ext-diff --no-textconv HEAD -- | sha256_stdin)"
serc_revision="${serc_commit:0:12}"
if [[ -n "$serc_status" ]]; then
    serc_revision="$serc_revision+dirty"
fi
corpus_revision="$(git -C "$corpus" rev-parse HEAD 2>/dev/null || echo unavailable)"
cargo_lock_sha256="$(sha256_file "$repo_root/Cargo.lock")"
serc_binary_sha256="$(sha256_file "$serc")"
{
    echo "timestamp_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "corpus=$corpus"
    echo "corpus_revision=$corpus_revision"
    echo "source_files=$source_count"
    echo "source_bytes=$source_bytes"
    echo "corpus_sha256=$corpus_sha256"
    echo "serc_revision=$serc_revision"
    echo "serc_commit=$serc_commit"
    echo "serc_dirty_diff_sha256=$serc_dirty_diff_sha256"
    echo "cargo_lock_sha256=$cargo_lock_sha256"
    echo "serc_binary=$serc"
    echo "serc_binary_sha256=$serc_binary_sha256"
    echo "serc_profile=$serc_profile"
    echo "serc_build_mode=$serc_build_mode"
    echo "pgo_profile=$pgo_profile"
    echo "pgo_profile_sha256=$pgo_profile_sha256"
    echo "pgo_target=$pgo_target"
    echo "serc_jobs=1"
    echo "cpython_jobs=1"
    echo "preflight_pyc_sha256=$serc_pyc_sha256"
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
    printf '%s\n' "$serc_status"
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
