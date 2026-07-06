# Serc benchmarks

Run benchmark commands from the workspace root with the pinned Rust toolchain and committed
lockfile.

## Criterion

Run the complete local benchmark suite with the profiling profile:

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks
```

Pass a benchmark filter after `--` to run a subset:

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks -- compile
```

Save a baseline, then compare a later run against it:

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks -- --save-baseline main
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks -- --baseline main
```

Additional Criterion options can be passed after `--`, for example:

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks -- --warm-up-time 3 --measurement-time 10 --sample-size 50
```

## CodSpeed simulation

Install the pinned CodSpeed CLI:

```console
cargo install cargo-codspeed --version 4.4.1 --locked
```

Build and run the benchmark suite in simulation mode:

```console
CARGO_INCREMENTAL=0 cargo codspeed build --locked -m simulation --profile profiling -p serc_benchmark --bench compiler --no-default-features --features "benchmarks,codspeed"
cargo codspeed run -m simulation -p serc_benchmark --bench compiler
```
