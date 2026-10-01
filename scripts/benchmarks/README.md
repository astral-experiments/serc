# End-to-end compiler benchmarks

The harness measures process startup, source discovery and reads, compilation at
optimization level 0, marshaling, and timestamp-invalidated `.pyc` writes. Both
compilers run sequentially: the preflight and timing use Serc `--jobs 1` and
CPython `-j 1`. Cache-file cleanup is untimed. For isolated phases, use the
[Criterion and corpus benchmarks](../../crates/serc_benchmark/README.md).

From the workspace root, with Git, the pinned Rust toolchain, and CPython 3.14.5
(magic `2b0e0d0a`):

```console
cargo install hyperfine --version 1.20.0 --locked
./scripts/benchmarks/setup.sh
PYTHON=/path/to/python3.14.5 ./scripts/benchmarks/run.sh
```

Setup clones the [pinned NumPy revision](setup.sh) to `target/benchmarks/numpy`,
where the runner requires a clean package tree. It validates the oracle identity
and builds release Serc once. Before timing, both compilers must successfully
compile every source and produce identical `.pyc` paths and full contents. This
workload-specific preflight does not replace the
[compatibility gate](../../docs/compatibility.md).

Hyperfine runs CPython first, with three warmups and at least ten measured runs
per compiler. Override these with `HYPERFINE_WARMUP` and `HYPERFINE_MIN_RUNS`.
On Linux, prefix the runner with `taskset -c <cpu>` to pin both compilers. Pass
another corpus and optional output path with:

```console
PYTHON=/path/to/python3.14.5 ./scripts/benchmarks/run.sh /path/to/project target/benchmarks/results/project.json
```

The runner deletes all `__pycache__` directories in the corpus, including before
each timed command. Results default to
`target/benchmarks/results/numpy.{json,md,metadata.txt}`. The metadata records
source and output hashes, source count and size, revisions and working-tree
state, toolchain and binary identities, build profile, worker counts, and
machine details. See [published results](../../BENCHMARKS.md).

## Profile-guided build

After running
[`scripts/build-pgo-release`](../../docs/development.md#profile-guided-release-build),
pass its merged profile to the runner. Use the same source and Rust toolchain
that generated it:

```console
PYTHON=/path/to/python3.14.5 \
SERC_PGO_PROFILE="$PWD/target/pgo-release/run-EXAMPLE/training.profdata" \
  ./scripts/benchmarks/run.sh
```

The runner records the profile path and SHA-256 alongside the usual benchmark
metadata. Unset `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` when using PGO.

## Compare two release builds

Build and preserve both executables before measuring. Use a disposable project
checkout: this harness recursively deletes its `__pycache__` directories before
every process. It keeps OS filesystem caches warm and includes discovery, reads,
compilation, first serialization, and cache writes in each elapsed time.

```console
python scripts/benchmarks/compare-releases.py \
  /path/to/serc-baseline /path/to/serc-optimized /path/to/project \
  target/benchmarks/results/release-comparison.json \
  --baseline-revision BASELINE_COMMIT_SHA \
  --python /path/to/python3.14.5 --cpus 2 --jobs 1 --warmups 5 --samples 50
```

Both binaries receive identical options: Python 3.14, optimization level 0,
timestamp invalidation, and the requested worker count. Choose an available CPU
or CPU list for `--cpus`. Each pair alternates baseline-first and optimized-first
order. Cleanup and validation are outside timing. Every run must reproduce the
baseline's complete output hashes, exit status, and diagnostics; `--python` also
requires byte-for-byte agreement with the exact CPython 3.14.5 oracle.

The JSON records every wall-clock and CPU-time sample, binary hashes, source and
output manifest hashes, project revision, commands, mean, sample standard
deviation, median, range, and a paired bootstrap 95% interval for the percentage
reduction in mean elapsed time. Use a fresh measurement series after exploratory
pilots, and compare existing Criterion workloads separately.

For a checkout containing deliberately invalid test fixtures, explicitly supply
`--expected-exit-code 1 --expected-output-count N`. The same diagnostics and all
`N` successful outputs are still checked on every run.
