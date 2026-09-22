# Benchmarks

Recorded September 20, 2026, on a Linux x86-64 VM with 32 AMD EPYC 7763 vCPUs, with both
compilers pinned to CPU 19: CPython 3.14.5, release Serc
[`1b57c9c`](https://github.com/astral-sh/serc/commit/1b57c9c33f7586a85d8a7ae2a5f3dd3b9e6a5cec),
Rust 1.97.0, and Hyperfine 1.20.0. The corpus was NumPy
[`v2.4.6`](https://github.com/numpy/numpy/tree/b832a09cf2a169c833dd2371e7c07aa00b293242):
505 Python files totaling 9,443,546 bytes.

| Run order | Measured runs per compiler | Serc | CPython `compileall` | CPython mean / Serc mean |
| --- | ---: | ---: | ---: | ---: |
| CPython first | 25 | 1.202 s ± 0.029 s | 1.717 s ± 0.026 s | 1.43× |
| Serc first | 25 | 1.186 s ± 0.009 s | 1.761 s ± 0.075 s | 1.49× |
| **Both orders** | **50** | **1.194 s ± 0.023 s** | **1.739 s ± 0.060 s** | **1.46×** |

Times are wall-clock means ± sample standard deviations; the combined row pools both orders.
Each order used five untimed warmups per compiler. See the
[raw samples](scripts/benchmarks/results/numpy-2026-09-20.csv).

Each fresh process discovers and reads sources, compiles at optimization level 0, marshals, and
writes timestamp-invalidated `.pyc` files sequentially. Cache-file cleanup occurs outside timing;
OS filesystem caches were not cleared. An untimed preflight verified all 505 output paths and complete
contents matched byte for byte. These results describe this workload and machine, not general
compatibility or performance guarantees.

To reproduce CPython-first on Linux with the [end-to-end harness](scripts/benchmarks/README.md),
substitute an available CPU for `19`:

```sh
./scripts/benchmarks/setup.sh
PYTHON=/path/to/python3.14.5 HYPERFINE_WARMUP=5 HYPERFINE_MIN_RUNS=25 \
  taskset -c 19 ./scripts/benchmarks/run.sh
```

Then reproduce Serc-first from the repository root, adjusting the binary path if Cargo uses a
custom target directory:

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
