# Benchmarks

## Project compilation

The [README chart](README.md#highlights) shows median wall times for the Linux
x86-64 releases of CPython 3.14.5 from
[python-build-standalone 20260602](https://github.com/astral-sh/python-build-standalone/releases/tag/20260602)
and [Serc 0.1.9](https://github.com/astral-experiments/serc/releases/tag/0.1.9).
All four plots use the same scale from zero to one second.

| Package sources | Python files | CPython 3.14.5 (16 workers) | Serc 0.1.9 (32 workers) |
| --- | ---: | ---: | ---: |
| [Airflow](https://github.com/apache/airflow/tree/cad6220034e87c586eb87e14b5a75f4b82984fd0/airflow-core/src/airflow) | 700 | 324 ms | 79 ms |
| [Django 5.2](https://github.com/django/django/tree/9e7cc2b628fe8fd3895986af9b7fc9525034c1b0/django) | 883 | 358 ms | 94 ms |
| [NumPy 2.4.6](https://github.com/numpy/numpy/tree/b832a09cf2a169c833dd2371e7c07aa00b293242/numpy) | 505 | 432 ms | 131 ms |
| [Transformers](https://github.com/huggingface/transformers/tree/ab1771c9e42891d893189978a8009426d70b4688/src/transformers) | 2,876 | 832 ms | 288 ms |

Both compilers had access to all 32 vCPUs on the same AMD EPYC Milan Linux VM.
Each input tree contained the tracked `.py` files from the linked package
directory, with their relative paths preserved. The timer covered process and
worker startup, source discovery, compilation, and writing `.pyc` files.

```sh
python3.14 -I -S -m compileall -q -f -j "$workers" -o 0 --invalidation-mode timestamp "$package"
serc compile --quiet --jobs "$workers" --python-version 3.14 --optimize 0 --invalidation-mode timestamp "$package"
```

We considered 4, 8, 16, and 32 workers. For each setting we took four measured
runs per package from an initial sweep and added six fresh runs. For each
compiler, we divided each package's ten-run median by its median with 32 workers
in the initial sweep. We chose the count with the lowest geometric mean of those
ratios, giving all four packages equal weight, and used that count throughout.
We shuffled the project and worker-count pairs each pass. In each comparison,
both compilers ran first equally often.

At the selected settings, all 4,964 `.pyc` paths and contents matched. The
chart uses ten fresh runs per compiler and package after two warmups. Cache-file
cleanup was untimed and filesystem caches stayed warm throughout. The
[raw timings](scripts/benchmarks/results/projects-2026-11-01.csv) distinguish
the initial sweep, refinement, and confirmation runs for the four candidates.

## Historical NumPy benchmark

The following NumPy results were recorded on Linux with an AMD EPYC 7763 and
CPython 3.14.5 in September 2026.

A few caveats apply:

- Performance depends on the hardware, operating system, filesystem, and Python
  sources being compiled. These results describe this workload and machine.
- Existing `.pyc` files are removed before every run, but OS filesystem caches
  are left warm. These measurements do not represent a cold filesystem cache or
  a parallel compilation workload.

Shorter bars indicate faster compilation.

### Bytecode compilation

Benchmarking compilation of NumPy 2.4.6: 505 Python files totaling 9,443,546
bytes. This is equivalent to compiling a package after installation, before its
first import.

Each command starts a fresh process, discovers and reads the sources, compiles
at optimization level 0, marshals the bytecode, and writes timestamp-invalidated
`.pyc` files. Both compilers process files sequentially. The reproduction
harness selects one worker explicitly with Serc `--jobs 1` and CPython `-j 1`.
Cache-file cleanup happens outside the timed command.

![Time to compile NumPy: Serc 1.194 seconds, CPython 1.739 seconds. Lower is better.](docs/assets/benchmarks/numpy-compilation.svg)

Serc compiled NumPy **1.46× faster** than CPython in this benchmark. The chart
shows mean wall-clock time across 50 runs per compiler; error bars show sample
standard deviations. An untimed preflight verified that all 505 output paths and
complete `.pyc` contents matched byte for byte. See the
[compatibility guide](docs/compatibility.md) for broader compatibility results.

<details>
<summary>Measurement details and raw results</summary>

Recorded September 20, 2026, on a Linux x86-64 VM with 32 AMD EPYC 7763 vCPUs,
with both compilers pinned to CPU 19:

- Serc
  [`1b57c9c`](https://github.com/astral-sh/serc/commit/1b57c9c33f7586a85d8a7ae2a5f3dd3b9e6a5cec),
  built with the release profile and Rust 1.97.0.
- CPython 3.14.5 and Hyperfine 1.20.0.
- NumPy
  [`v2.4.6`](https://github.com/numpy/numpy/tree/b832a09cf2a169c833dd2371e7c07aa00b293242).

The benchmark was run in both command orders, with five untimed warmups and 25
measured runs per compiler in each order. Times are means ± sample standard
deviations; the combined row pools both orders. Speedup is the CPython mean
divided by the Serc mean.

| Run order       | Runs per compiler |                  Serc |  CPython `compileall` |   Speedup |
| --------------- | ----------------: | --------------------: | --------------------: | --------: |
| CPython first   |                25 |     1.202 s ± 0.029 s |     1.717 s ± 0.026 s |     1.43× |
| Serc first      |                25 |     1.186 s ± 0.009 s |     1.761 s ± 0.075 s |     1.49× |
| **Both orders** |            **50** | **1.194 s ± 0.023 s** | **1.739 s ± 0.060 s** | **1.46×** |

The chart and table use the checked-in
[raw samples](scripts/benchmarks/results/numpy-2026-09-20.csv).

</details>

### Reproduction

The [benchmark harness](scripts/benchmarks/README.md) wraps
[`hyperfine`](https://github.com/sharkdp/hyperfine) and requires Git, the pinned
Rust toolchain, CPython 3.14.5, and Hyperfine 1.20.0. It builds release Serc and
verifies byte-for-byte output compatibility before timing.

From the repository root, set up the pinned NumPy corpus and run the benchmark.
On Linux, substitute an available CPU for `19`:

```sh
./scripts/benchmarks/setup.sh
PYTHON=/path/to/python3.14.5 HYPERFINE_WARMUP=5 HYPERFINE_MIN_RUNS=25 \
  taskset -c 19 ./scripts/benchmarks/run.sh
```

Results are written to `target/benchmarks/results/numpy.{json,md,metadata.txt}`.
The published results use the Serc revision listed above; running the harness
benchmarks your checkout.

<details>
<summary>Repeat with Serc first</summary>

The harness runs CPython first. To repeat the measurement in the opposite order
after running the harness, use the following command from the repository root.
Adjust the CPU and binary path if needed, including when Cargo uses a custom
target directory.

```sh
(
  export BENCH_PYTHON=/path/to/python3.14.5
  export BENCH_SERC="$PWD/target/release/serc"
  export BENCH_CORPUS="$PWD/target/benchmarks/numpy/numpy"
  BENCH_CPU=19
  taskset -c "$BENCH_CPU" hyperfine --warmup 5 --min-runs 25 \
    --prepare 'find "$BENCH_CORPUS" -type d -name __pycache__ -prune -exec rm -rf {} +' \
    --prepare 'find "$BENCH_CORPUS" -type d -name __pycache__ -prune -exec rm -rf {} +' \
    '"$BENCH_SERC" compile --quiet --jobs 1 "$BENCH_CORPUS"' \
    '"$BENCH_PYTHON" -I -S -m compileall -q -f -j 1 -o 0 --invalidation-mode timestamp "$BENCH_CORPUS"'
)
```

</details>

For benchmarks of individual compiler phases, see the
[Criterion and corpus benchmarks](crates/serc_benchmark/README.md).
