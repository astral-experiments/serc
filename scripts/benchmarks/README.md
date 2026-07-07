# End-to-end compiler benchmarks

This harness compares the complete command-line workflow for CPython and Serc on a real Python
package. Each measured command starts a fresh process, discovers every `.py` file, reads it,
compiles it at optimization level 0, marshals it, and writes a timestamp-invalidated `.pyc` file.
Both compilers run sequentially. Hyperfine removes generated `__pycache__` directories before
each command, outside the timed interval.

Serc's Criterion and corpus benchmarks remain better tools for measuring isolated compiler
phases. This harness intentionally measures startup, discovery, filesystem I/O, compilation,
marshaling, and cache-file generation together.

## Prerequisites

- the pinned Rust toolchain;
- Hyperfine;
- an exact CPython 3.14.5 executable with magic number `2b0e0d0a`;
- Git.

Install the pinned Hyperfine release with:

```console
cargo install hyperfine --version 1.20.0 --locked
```

The runner validates the CPython implementation, version, and magic number before benchmarking.

## NumPy benchmark

Set up the corpus once:

```console
./scripts/benchmarks/setup.sh
```

The setup script shallow-clones NumPy `v2.4.6` at commit
`b832a09cf2a169c833dd2371e7c07aa00b293242` under `target/benchmarks/numpy`. Corpus setup is
never part of a measured command. The default runner rejects tracked changes or untracked
filesystem artifacts in the measured NumPy package tree.

Run the benchmark:

```console
PYTHON=/path/to/python3.14.5 ./scripts/benchmarks/run.sh
```

The runner builds Serc once with the release profile, preflights both commands, then runs
Hyperfine with three warmups and at least ten measured runs. The untimed preflight hashes the
relative paths and complete contents of every generated `.pyc` and requires Serc's aggregate
to match CPython's. This does not add work to either measured command.

The runner writes machine-readable JSON, a Markdown table, and a reproducibility sidecar to
`target/benchmarks/results/numpy.{json,md,metadata.txt}`. The sidecar records the full Serc
commit, a hash of its tracked working-tree diff, the Cargo lockfile and resolved compiler binary
hashes, the actual Cargo profile, an aggregate corpus hash, the bytecode preflight hash, source
count and size, CPython build, Rust toolchain, Hyperfine version, platform, and available CPU
metadata. The status sections retain human-readable tracked and untracked state.

Override the sampling defaults with `HYPERFINE_WARMUP` and `HYPERFINE_MIN_RUNS`:

```console
HYPERFINE_WARMUP=5 HYPERFINE_MIN_RUNS=20 ./scripts/benchmarks/run.sh
```

To benchmark another source tree, pass the corpus and optional JSON destination:

```console
./scripts/benchmarks/run.sh /path/to/project target/benchmarks/results/project.json
```

The corpus is expected to be disposable with respect to generated Python bytecode: the runner
removes `__pycache__` directories before every Hyperfine command.

## Correctness

The preflight requires both commands to compile the entire corpus successfully and produce
byte-identical `.pyc` trees. It is an exact check for the selected performance corpus, but it does
not replace the broader frozen compatibility corpus. Inspect an arbitrary corpus in detail with:

```console
cargo run --locked -p serc --example compare_cpython_corpus -- \
  --python /path/to/cpython-3.14.5 \
  --expected-files 505 \
  target/benchmarks/numpy/numpy
```

NumPy is a performance workload, not the frozen compatibility corpus, so this reporting command
intentionally omits `--require-all`. Run the authoritative pinned compatibility gate when changing
the compiler, assembler, marshal encoder, target, or dependencies:

```console
./scripts/check-cpython-compatibility /path/to/cpython-3.14.5
```
