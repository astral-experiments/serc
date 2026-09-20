# Benchmarks

These benchmarks were recorded on September 20, 2026, on a Linux x86-64 virtual machine
exposing 32 AMD EPYC 7763 vCPUs. Both compilers were pinned to logical CPU 19.
The versions were:

- CPython 3.14.5
- Serc [`1b57c9c`](https://github.com/astral-sh/serc/commit/1b57c9c33f7586a85d8a7ae2a5f3dd3b9e6a5cec), built in the release profile
- Hyperfine 1.20.0
- Rust 1.97.0
- NumPy [`v2.4.6`](https://github.com/numpy/numpy/tree/b832a09cf2a169c833dd2371e7c07aa00b293242)

Benchmark performance varies across operating systems, machines, and workloads. This benchmark
compares the complete sequential command-line workflow on NumPy's 505 Python files (9,443,546
bytes). Each measured command starts a fresh process, discovers and reads the sources, compiles
them at optimization level 0, marshals them, and writes timestamp-invalidated `.pyc` files.
Hyperfine removes generated `__pycache__` directories before each command, outside the timed
interval. The operating system's filesystem caches were not cleared.

## NumPy

| Run order | Measured runs per compiler | Serc | CPython `compileall` | CPython mean / Serc mean |
| --- | ---: | ---: | ---: | ---: |
| CPython first | 25 | 1.202 s ± 0.029 s | 1.717 s ± 0.026 s | 1.43× |
| Serc first | 25 | 1.186 s ± 0.009 s | 1.761 s ± 0.075 s | 1.49× |
| **Both orders** | **50** | **1.194 s ± 0.023 s** | **1.739 s ± 0.060 s** | **1.46×** |

Times are wall-clock means ± the sample standard deviation of the individual runs. The last
column divides the CPython mean by the Serc mean; the combined row pools all 50 runs per
compiler. Each order used five untimed warmups per compiler, followed by 25 measured runs per
compiler. See the [raw timing samples](scripts/benchmarks/results/numpy-2026-09-20.csv).

The stock [end-to-end benchmark harness](scripts/benchmarks/README.md) runs CPython first.
To reproduce that order with the published sampling settings, set up the corpus, then run the
following; on Linux, insert `taskset -c <cpu>` immediately before `./scripts/benchmarks/run.sh`
to pin both compilers to a single available logical CPU:

```sh
./scripts/benchmarks/setup.sh
PYTHON=/path/to/python3.14.5 HYPERFINE_WARMUP=5 HYPERFINE_MIN_RUNS=25 \
  ./scripts/benchmarks/run.sh
```

The second run reversed the order of the same timed commands and kept the cleanup, CPU pin,
and sampling settings unchanged. CPython used `-I -S -m compileall -q -f -j 1 -o 0
--invalidation-mode timestamp`; Serc used `serc compile --quiet`.
After running the stock harness above, the reverse order can be reproduced on Linux from the
repository root with the following. Replace CPU 19 with a logical CPU available on your machine;
if Cargo uses a custom target directory, also adjust the Serc executable path.

```sh
(
  export BENCH_PYTHON=/path/to/python3.14.5
  export BENCH_SERC="$PWD/target/release/serc"
  export BENCH_CORPUS="$PWD/target/benchmarks/numpy/numpy"
  BENCH_CPU=19
  taskset -c "$BENCH_CPU" hyperfine --warmup 5 --min-runs 25 \
    --prepare 'find "$BENCH_CORPUS" -type d -name __pycache__ -prune -exec rm -rf {} +' \
    --prepare 'find "$BENCH_CORPUS" -type d -name __pycache__ -prune -exec rm -rf {} +' \
    '"$BENCH_SERC" compile --quiet "$BENCH_CORPUS"' \
    '"$BENCH_PYTHON" -I -S -m compileall -q -f -j 1 -o 0 --invalidation-mode timestamp "$BENCH_CORPUS"'
)
```

Before timing, the stock runner verified that each compiler produced 505 `.pyc` files and that
the relative paths and full contents matched byte for byte. This is a correctness check for the
NumPy workload; it does not replace the project's broader compatibility corpus.
