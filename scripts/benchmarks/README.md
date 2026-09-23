# End-to-end compiler benchmarks

The harness measures process startup, source discovery and reads, compilation at optimization
level 0, marshaling, and timestamp-invalidated `.pyc` writes. Both compilers run sequentially:
the preflight and timing use Serc `--jobs 1` and CPython `-j 1`. Cache-file cleanup is untimed.
For isolated phases, use the
[Criterion and corpus benchmarks](../../crates/serc_benchmark/README.md).

From the workspace root, with Git, the pinned Rust toolchain, and CPython 3.14.5 (magic
`2b0e0d0a`):

```console
cargo install hyperfine --version 1.20.0 --locked
./scripts/benchmarks/setup.sh
PYTHON=/path/to/python3.14.5 ./scripts/benchmarks/run.sh
```

Setup clones the [pinned NumPy revision](setup.sh) to `target/benchmarks/numpy`, where the runner
requires a clean package tree. It validates the oracle identity and builds release Serc once.
Before timing, both compilers must successfully compile every source and produce identical
`.pyc` paths and full contents. This workload-specific preflight does not replace the
[compatibility gate](../../docs/compatibility.md).

Hyperfine runs CPython first, with three warmups and at least ten measured runs per compiler.
Override these with `HYPERFINE_WARMUP` and `HYPERFINE_MIN_RUNS`. On Linux, prefix the runner
with `taskset -c <cpu>` to pin both compilers. Pass another corpus and optional output path with:

```console
PYTHON=/path/to/python3.14.5 ./scripts/benchmarks/run.sh /path/to/project target/benchmarks/results/project.json
```

The runner deletes all `__pycache__` directories in the corpus, including before each timed command.
Results default to `target/benchmarks/results/numpy.{json,md,metadata.txt}`. The metadata records
source and output hashes, source count and size, revisions and working-tree state, toolchain and
binary identities, build profile, worker counts, and machine details. See
[published results](../../BENCHMARKS.md).

## Profile-guided build

After running [`scripts/build-pgo-release`](../../docs/development.md#profile-guided-release-build),
pass its merged profile to the runner. Use the same source and Rust toolchain that generated it:

```console
PYTHON=/path/to/python3.14.5 \
SERC_PGO_PROFILE="$PWD/target/pgo-release/run-EXAMPLE/training.profdata" \
  ./scripts/benchmarks/run.sh
```

The runner builds the CLI with this profile and records `release-pgo`, the profile path, and
its SHA-256 in the result metadata. It retains the complete bytecode preflight, one-worker
workload, and untimed cache cleanup. Omitting `SERC_PGO_PROFILE` keeps the ordinary release
build. Unset `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` for the PGO mode.
