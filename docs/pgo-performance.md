# PGO release performance: Django and NumPy

On October 1, 2026, a profile-guided release compiled the complete Django package
and wrote its `.pyc` files using **10.09% less wall time** than an ordinary release. An independent
repeat measured **11.00%**. Both 95% intervals lie above 5%. NumPy and four-worker
Django compilation also improved by about 11%.

This is a **PGO build improvement**, using the existing pinned training recipe.
Both executables use Rust source from baseline
`6fc817a30558585a5879ddd869b2ccaef889a787`, fetched from `origin/main` before work.
There are no Rust source or dependency differences between the executables.
The new implementation provides a manually dispatched [PGO release job](../.github/workflows/pgo.yml)
that verifies, benchmarks, and packages the optimized Linux host binary, plus a
[paired comparison harness](../scripts/benchmarks/compare-builds.py).
Ordinary `cargo build --release` and portable PyPI wheels are unchanged.

PGO has a measurable tradeoff: two small import phase benchmarks regress by
7.60% and 5.99%. Those results are detailed below; this is not a claim that every
workload improves.

## End-to-end results

Each row has **30 measured pairs**, preceded by **10 warmup pairs**. Times are
seconds, shown as mean ± sample standard deviation. Reduction is
`100 × (1 − optimized_mean / baseline_mean)`. Intervals use 10,000 paired bootstrap
resamples, seed 0. All observations, including outliers, are retained.

| Workload | Ordinary release (s) | PGO release (s) | Wall-time reduction | Paired 95% interval |
| --- | ---: | ---: | ---: | ---: |
| Django, 1 worker | 1.094750 ± 0.040595 | 0.984285 ± 0.044335 | 10.09% | 8.44–11.65% |
| Django, independent repeat | 1.100765 ± 0.045270 | 0.979712 ± 0.019883 | 11.00% | 9.96–12.29% |
| NumPy, 1 worker | 1.207449 ± 0.048954 | 1.073120 ± 0.039488 | 11.13% | 9.64–12.75% |
| Django, 4 workers | 0.405526 ± 0.022442 | 0.360331 ± 0.024704 | 11.14% | 9.13–13.20% |

The [raw results](../scripts/benchmarks/results/pgo-2026-10-01/) include every
measured pair, medians, ranges, executable hashes, source/output manifest hashes,
commands, CPU affinity, and environment metadata.

- Django 5.2.6: `75c4403f07b8ad25893f7832dbe8fc6814b53b2d`.
  All **883** `.py` files in the shipped `django/` package, **5,653,893 bytes**,
  with no exclusions. Deliberately invalid repository test fixtures are outside
  this package.
- NumPy 2.4.6: `b832a09cf2a169c833dd2371e7c07aa00b293242`.
  All **505** `.py` files in `numpy/`, **9,443,546 bytes**.
- Both projects were held out from PGO training. Training used the repository's
  nine pinned projects: 564 files, 6,485,134 bytes, plus the pinned Ruff corpus.

Every observation starts a new CLI process, discovers and reads every source,
compiles each file and serializes it once, writes/renames each resulting `.pyc`,
and exits. The timer includes process startup, all compilation/serialization,
and actual filesystem writes. It does not time repeated serialization of an
already compiled module. Writes use the normal workflow without an extra `fsync`.

Before **every** observation, the harness removes all `__pycache__` directories
and reads all sources to warm the page cache. Cleanup, warming, and verification
are outside the timer. It alternates baseline/optimized and optimized/baseline
order, checks complete output paths and bytes after every invocation, and checks
that source contents and mtimes remain unchanged. Runtime options match within
each comparison: quiet mode, Python 3.14, optimization 0, timestamp invalidation,
and either one or four workers. No builds, tests, profilers, or other benchmarks
ran concurrently with these measurements.

The same Debian 13 Linux machine was used throughout: Intel Xeon Platinum 8573C,
5 visible vCPUs, 4-CPU cgroup quota, 16 GiB memory, `/workspace` on overlayfs,
kernel 6.18.44, glibc 2.41. Serial runs were pinned to CPU 0; the parallel run
used CPUs 0–3. Inactive debug-build file pages were released before final
measurements to avoid memory pressure; the warmup pairs warmed both executables
and the source corpus equally. See [machine metadata](../scripts/benchmarks/results/pgo-2026-10-01/machine.json).

## Builds and exact commands

Commands were run from `/workspace/serc`. The machine's activation script sets
Rust 1.97.0 (LLVM 22.1.6), `CARGO_BUILD_JOBS=4`, `RUST_TEST_THREADS=4`,
`RAYON_NUM_THREADS=4`, and the exact CPython oracle paths. No target-native CPU
flags were used. Both CLI builds use the locked dependencies, explicit
`x86_64-unknown-linux-gnu` target, release profile (`opt-level=3`, fat LTO,
one codegen unit, stripped symbols), panic=unwind, and incremental=0.
The deliberate build treatment is `-Cprofile-use`; runtime options are identical.

```sh
git fetch origin main
git rev-parse origin/main
# 6fc817a30558585a5879ddd869b2ccaef889a787
source /workspace/serc-environment/activate.sh
rustup component add llvm-tools-preview
./scripts/build-pgo-release "$SERC_TEST_PYTHON" --output-dir work/performance/pgo

git clone --depth 1 --branch 5.2.6 https://github.com/django/django.git work/performance/django
./scripts/benchmarks/setup.sh
```

The recipe produced `work/performance/pgo/run-j61l5u07/`. Its
[complete build manifest](../scripts/benchmarks/results/pgo-2026-10-01/build-manifest.json)
records all exact build, training, oracle, and profile-merge commands and project
pins. In particular, the control and final CLI invocations were:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_PANIC=unwind CARGO_PROFILE_DEV_PANIC=unwind \
  CARGO_TARGET_DIR=/workspace/serc/work/performance/pgo/control-target \
  CARGO_ENCODED_RUSTFLAGS=''   cargo build --locked --profile release --target \
  x86_64-unknown-linux-gnu   --message-format=json-render-diagnostics -p serc-cli --bin serc

CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_PANIC=unwind CARGO_PROFILE_DEV_PANIC=unwind \
  CARGO_TARGET_DIR=/workspace/serc/work/performance/pgo/use-target \
  CARGO_ENCODED_RUSTFLAGS=-Cprofile-use=/workspace/serc/work/performance/pgo/run-j61l5u07/training.profdata \
  cargo build --locked --profile release --target x86_64-unknown-linux-gnu \
  --message-format=json-render-diagnostics -p serc-cli --bin serc
```

Artifact SHA-256 identities:

| Artifact | SHA-256 |
| --- | --- |
| Ordinary `serc-control` | `6d2b4dee4068650f7a4123a6cf7a5a9b610de406a3702a4bee2ed6b81a6da96e` |
| Optimized `serc` | `7df19148ebdc246e6e2b13ad0083ede435f6980bcb99a3006772fbce09a68461` |
| `training.profdata` | `e6e216fe3ec24817e747aff0c7e4cc8417bc08e3716783836da087080ed69fae` |

Exact end-to-end benchmark commands:

```sh
python3 scripts/benchmarks/compare-builds.py   work/performance/pgo/run-j61l5u07/serc-control \
  work/performance/pgo/run-j61l5u07/serc   work/performance/django/django \
  work/performance/pgo-django.json   --samples 30 --warmups 10 --jobs 1 --cpu 0 \
  --baseline-revision 6fc817a30558585a5879ddd869b2ccaef889a787   --optimized-revision \
  6fc817a30558585a5879ddd869b2ccaef889a787

python3 scripts/benchmarks/compare-builds.py   work/performance/pgo/run-j61l5u07/serc-control \
  work/performance/pgo/run-j61l5u07/serc   work/performance/django/django \
  work/performance/pgo-django-repeat.json   --samples 30 --warmups 10 --jobs 1 --cpu 0 \
  --baseline-revision 6fc817a30558585a5879ddd869b2ccaef889a787   --optimized-revision \
  6fc817a30558585a5879ddd869b2ccaef889a787

python3 scripts/benchmarks/compare-builds.py   work/performance/pgo/run-j61l5u07/serc-control \
  work/performance/pgo/run-j61l5u07/serc   target/benchmarks/numpy/numpy \
  work/performance/pgo-numpy.json   --samples 30 --warmups 10 --jobs 1 --cpu 0 \
  --baseline-revision 6fc817a30558585a5879ddd869b2ccaef889a787   --optimized-revision \
  6fc817a30558585a5879ddd869b2ccaef889a787

taskset -c 0,1,2,3 python3 scripts/benchmarks/compare-builds.py \
  work/performance/pgo/run-j61l5u07/serc-control work/performance/pgo/run-j61l5u07/serc \
  work/performance/django/django work/performance/pgo-django-parallel.json   --samples 30 \
  --warmups 10 --jobs 4   --baseline-revision 6fc817a30558585a5879ddd869b2ccaef889a787 \
  --optimized-revision 6fc817a30558585a5879ddd869b2ccaef889a787
```

## Correctness and compatibility

- **7,393 workspace tests passed**, including the pinned Python 3.12.14,
  3.13.15, 3.14.5, and 3.15.0rc2 oracles. Formatting, strict workspace Clippy,
  and Python style checks passed.
- The PGO build's strict instrumented Ruff compatibility gate passed on **2,470
  files**: 2,246 exact accepted outputs and 224 CPython-rejected inputs, comprising
  222 matched rejections and two pre-existing expected compiler failures.
  No byte mismatches or new failure allowances. Ruff dependencies and the corpus
  retain pin `0be08a206f9c3180afd3e93bcc792ed5cb1f4db1`.
- Instrumented and final PGO CLI builds matched ordinary-release complete `.pyc`
  files for all 564 training sources with both one and four workers.
- Ordinary and PGO releases matched all **883 Django outputs in each of 36
  configurations**: Python 3.12/3.13/3.14/3.15 × optimization 0/1/2 × timestamp,
  checked-hash, and unchecked-hash invalidation. This is 31,788 paired full-file
  comparisons. The [results](../scripts/benchmarks/results/pgo-2026-10-01/release-output-checks.json)
  retain every command and output manifest hash.
- All 883 default Django `.pyc` files also matched the exact CPython 3.14.5 oracle.
  Every warmup and measured end-to-end run checked complete output equality.
- The PGO training manifest retains **five existing baseline differences against
  CPython** (559/564 exact), in Astropy, Sphinx, and Warehouse. PGO does not add
  differences; this report does not claim universal CPython equivalence.
- The new comparison harness was smoke-tested with identical binaries and a
  deliberately corrupting wrapper; it accepted the former and rejected changed
  `.pyc` bytes. The workflow passed Actionlint 1.7.7 after expanding its existing
  YAML path alias for that older linter. The release recipe was exercised locally;
  the new GitHub job has not been dispatched.

```sh
source /workspace/serc-environment/activate.sh
CARGO_BUILD_JOBS=2 cargo test --workspace --all-targets --all-features --locked
cargo fmt --all --check
CARGO_BUILD_JOBS=2 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
./scripts/check-python-style
```

## Existing benchmark regressions

All **51 existing Criterion cases** were run on ordinary and profile-use release
builds, pinned to CPU 0, with 30 samples, 0.5 s warmup, and 1 s measurement each.
These are phase/regression checks, **not evidence for the end-to-end gain**.
The serialization cases compile a fresh module for each iteration; they do not
reuse serialization state. These phase benchmarks do not include filesystem I/O.

Seven cases initially showed a mean slowdown above 5%. Each was rerun with
100 samples, 1 s warmup, and 5 s measurement. Three uncertain cases were also
run in reverse order. The import cases were then checked in **eight alternating
process pairs**, 50 samples per invocation, 0.5 s warmup, and 1 s measurement.
The paired result, which controls the observed order sensitivity, was:

| Existing case | Ordinary mean ± between-run SD | PGO mean ± between-run SD | Slowdown | Paired 95% interval |
| --- | ---: | ---: | ---: | ---: |
| `compile/micro_imports` | 18.603 ± 0.161 µs | 20.016 ± 0.436 µs | 7.60% | 6.47–8.74% |
| `compile_pyc/micro_imports` | 20.463 ± 0.302 µs | 21.688 ± 0.265 µs | 5.99% | 4.36–7.48% |

These are retained regressions, roughly 1.4 µs and 1.2 µs per small synthetic
module. The longer checks also found smaller slowdowns for `compile/micro_literals`
(+2.08%) and `compile_pyc/constants` (+2.28% in the reverse check). The initial
large-dataset, branch, and match-control-flow slowdowns did not repeat.
The raw [initial](../scripts/benchmarks/results/pgo-2026-10-01/criterion.json),
[longer](../scripts/benchmarks/results/pgo-2026-10-01/criterion-rechecks.json),
[reverse-order](../scripts/benchmarks/results/pgo-2026-10-01/criterion-reverse.json),
and [paired import](../scripts/benchmarks/results/pgo-2026-10-01/criterion-imports-paired.json)
results are retained rather than selecting only favorable measurements.

Benchmark executables were built with:

```sh
CARGO_TARGET_DIR="$PWD/work/performance/pgo/control-target" CARGO_BUILD_JOBS=1 \
  CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_PANIC=unwind   cargo bench --profile release \
  --locked --target x86_64-unknown-linux-gnu   -p serc_benchmark --bench compiler \
  --no-default-features --features benchmarks   --no-run --message-format=json

CARGO_TARGET_DIR="$PWD/work/performance/pgo/bench-use-target" CARGO_BUILD_JOBS=2 \
  CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_PANIC=unwind \
  CARGO_ENCODED_RUSTFLAGS="-Cprofile-use=$PWD/work/performance/pgo/run-j61l5u07/training.profdata" \
  cargo bench --profile release --locked --target x86_64-unknown-linux-gnu   -p serc_benchmark \
  --bench compiler --no-default-features --features benchmarks   --no-run --message-format=json
```

The resulting `compiler` executables were retained as
`work/performance/bench-pgo-control` and `work/performance/bench-pgo-optimized`.
The complete-suite commands were:

```sh
taskset -c 0 work/performance/bench-pgo-control --bench   --warm-up-time 0.5 \
  --measurement-time 1 --sample-size 30 --save-baseline pgo-control
taskset -c 0 work/performance/bench-pgo-optimized --bench   --warm-up-time 0.5 \
  --measurement-time 1 --sample-size 30 --baseline pgo-control
```

The longer runs used `--warm-up-time 1 --measurement-time 5 --sample-size 100`
and filtered the seven case IDs recorded in `criterion-rechecks.json`.
The paired import commands used the filter
`'^(compile/micro_imports|compile_pyc/micro_imports)$'`,
`--warm-up-time 0.5 --measurement-time 1 --sample-size 50 --noplot`, and separate
`--save-baseline imports-{baseline|optimized}-{0..7}` names, alternating AB/BA.
Their reported interval resamples the eight pairs of per-invocation Criterion
means, rather than treating all inner-loop iterations as independent process runs.
