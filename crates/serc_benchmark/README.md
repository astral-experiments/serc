# Serc benchmarks

Run benchmark commands from the workspace root with the pinned Rust toolchain
and committed lockfile.

## Criterion

The benchmark groups measure fixed and generated workloads through Serc's public
API:

| Group                                                         | Timed work                                                                     | Per-iteration setup and cleanup                          |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------- |
| `compile`                                                     | Compile source into a fresh module                                             | Module cleanup is outside timing                         |
| `marshal`                                                     | Marshal a newly compiled module exactly once                                   | Compilation and module/output cleanup are outside timing |
| `compile_pyc`                                                 | Compile source, encode one timestamp PYC in memory, and drop the module/output | All work is timed; no disk I/O                           |
| `compile_pyc_O1`, `compile_pyc_O2`                            | The same one-shot PYC workflow at optimization levels 1 and 2                  | All work is timed; no disk I/O                           |
| `compile_pyc_scaling_3_12` through `compile_pyc_scaling_3_15` | One-shot PYC generation for growing functions on each Python target            | Source generation is outside timing                      |

`marshal` compiles a fresh module during batch setup for every iteration,
including warmup. It does not reuse or clone a previously serialized module.
This makes first-serialization preparation visible even if an implementation
caches work for subsequent calls. `compile_pyc` also measures preparation moved
into compilation, and includes cleanup costs that phase-isolated measurements
exclude. It covers all compile and marshal workloads, including the large
fixture. The original groups use optimization level 0; `compile_pyc_O1` and
`compile_pyc_O2` use the same workloads with explicit optimization levels.

The scaling groups compile functions with 32 and 128 sequential exception
regions, comprehensions, or async manager blocks, plus mapping patterns with
32 and 128 captured values. Each shape runs on Python 3.12 through 3.15 at
optimization level 0, including compilation, PYC encoding, and cleanup.

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks
```

Append `-- compile/`, `-- marshal/`, `-- compile_pyc/`, `-- compile_pyc_O1/`, or
`-- compile_pyc_O2/` to filter by group, `-- --save-baseline main` to save a
baseline, or `-- --baseline main` to compare against it. Other Criterion options
follow `--`.

## CodSpeed simulation

```console
cargo install cargo-codspeed --version 4.4.1 --locked
CARGO_INCREMENTAL=0 cargo codspeed build --locked -m simulation --profile profiling -p serc_benchmark --bench compiler --no-default-features --features "benchmarks,codspeed"
cargo codspeed run -m simulation -p serc_benchmark --bench compiler
```

## CPython corpus comparison

The corpus benchmark uses one long-lived process per engine, with source
discovery and reads outside timing. It supports three phases:

| `--phase`         | Timed work                                 |
| ----------------- | ------------------------------------------ |
| `compiler_core`   | Compile source into code objects           |
| `compile_marshal` | Compile and marshal                        |
| `marshal_only`    | Marshal retained, precompiled code objects |

The corpus example's `marshal_only` phase reuses modules across samples. Use
`compile_marshal` to compare the usual compile-once/serialize-once workflow.

```console
cargo run --locked --profile profiling -p serc --example benchmark_cpython_corpus -- \
  --python /path/to/python3.14.5 --phase compiler_core \
  --warmups 5 --samples 21 --cooldown-ms 5000 \
  --output target/serc-compiler-core.tsv path/to/corpus
```

Pass one or more source roots. Before timing, Serc must compile every
CPython-accepted source and match its marshal payload byte for byte.
CPython-rejected sources are excluded. Each warmup or measured block alternates
engine order (ABBA/BAAB), with a cooldown before each observation. The TSV
records wall and process CPU times, corpus hashes, build profile, revisions,
working-tree state, and executable identities. Run each phase separately with a
distinct output path.

For complete CLI timing, including startup and `.pyc` writes, use the
[end-to-end harness](../../scripts/benchmarks/README.md).
