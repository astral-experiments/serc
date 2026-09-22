# Serc benchmarks

Run benchmark commands from the workspace root with the pinned Rust toolchain and committed
lockfile.

## Criterion

The `compile` and `marshal` groups measure fixed workloads through Serc's public API:

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks
```

Append `-- compile` or `-- marshal` to filter, `-- --save-baseline main` to save a baseline, or
`-- --baseline main` to compare against it. Other Criterion options follow `--`.

## CodSpeed simulation

```console
cargo install cargo-codspeed --version 4.4.1 --locked
CARGO_INCREMENTAL=0 cargo codspeed build --locked -m simulation --profile profiling -p serc_benchmark --bench compiler --no-default-features --features "benchmarks,codspeed"
cargo codspeed run -m simulation -p serc_benchmark --bench compiler
```

## CPython corpus comparison

The corpus benchmark uses one long-lived process per engine, with source discovery and reads
outside timing. It supports three phases:

| `--phase` | Timed work |
| --- | --- |
| `compiler_core` | Compile source into code objects |
| `compile_marshal` | Compile and marshal |
| `marshal_only` | Marshal precompiled code objects |

```console
cargo run --locked --profile profiling -p serc --example benchmark_cpython_corpus -- \
  --python /path/to/python3.14.5 --phase compiler_core \
  --warmups 5 --samples 21 --cooldown-ms 5000 \
  --output target/serc-compiler-core.tsv path/to/corpus
```

Pass one or more source roots. Before timing, Serc must compile every CPython-accepted source
and match its marshal payload byte for byte. CPython-rejected sources are excluded. Each warmup
or measured block alternates engine order (ABBA/BAAB), with a cooldown before each observation.
The TSV records wall and process CPU times, corpus hashes, build profile, revisions, working-tree
state, and executable identities. Run each phase separately with a distinct output path.

For complete CLI timing, including startup and `.pyc` writes, use the
[end-to-end harness](../../scripts/benchmarks/README.md).
