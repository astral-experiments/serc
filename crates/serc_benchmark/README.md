# Serc benchmarks

This crate provides stable performance baselines for Serc's public compile and marshal seams.
It is a non-published harness crate; it does not add a benchmark-only API to the compiler.

## Workloads and provenance

The suite embeds four project-owned fixtures in a fixed order. They were adopted at Serc commit
`a83330b97c6b3d2f9830b4d0890ee7a5a0125173` and are validated byte-for-byte against CPython tag
`v3.14.5`, commit `5607950ef232dad16d75c0cf53101d9649d89115`. The three files under
`fixtures/benchmark` materialize exact source literals that were already checked into Serc's
compatibility tests at the adoption commit; the standalone files are now their single source of
truth.

| Stable case ID | Source | Logical filename | Bytes | Construct coverage |
| --- | --- | --- | ---: | --- |
| `context_managers` | `crates/serc/tests/fixtures/execution/context_managers.py` | `benchmark/context_managers.py` | 775 | classes, nested contexts, suppression, loop exits |
| `annotation_scopes` | `crates/serc/tests/fixtures/benchmark/annotation_scopes.py` | `annotation_scopes.py` | 479 | nested code, generics, aliases, annotation scopes |
| `match_control_flow` | `crates/serc/tests/fixtures/benchmark/match_control_flow.py` | `match.py` | 1,169 | match cases and guards, `except*`, comprehension |
| `loop_async_edges` | `crates/serc/tests/fixtures/benchmark/loop_async_edges.py` | `loop_tail.py` | 2,019 | sync/async loops, cleanup, generators, boolean tails |

These 4,442 bytes are a small construct-rich starter set, not a representative large-module
corpus. A source-byte or logical-filename change is semantic; changing either, the case order, or a
stable ID requires a new benchmark identity and baseline rather than silently continuing the
existing series. The filenames are intentionally not normalized because they are serialized into
code objects.

## Measurement boundary

The suite reports exactly eight stable measurements: `compile/<case>` and `marshal/<case>` for
each table row. `compile` times one `serc::compile` call, including normalization, Ruff parsing,
lowering, all assembler passes, layout, and module construction. `marshal` compiles once outside
the timed region and then times one fresh `CompiledModule::marshal` call. Both groups report source
bytes as throughput. Both use explicit `BatchSize::LargeInput` batching: the setup closure is empty,
output destruction remains outside the timed region, and retained heap-owning outputs are bounded
by Criterion's large-value policy.

Each case is compiled successfully once before measurement. Complete marshal-byte equality is
checked separately by Serc's ordinary compatibility test and the frozen CPython corpus gate;
CPython invocation, fixture discovery, assertions, and output hashing are never timed.

There is deliberately no isolated assembler benchmark. Serc has no stable public prepared-IR
seam, and exposing one would expand the compiler's compatibility surface. There are also no
compile-plus-marshal, batch, parser-only, large-fixture, allocator, memory, or walltime series in
this baseline.

## Local Criterion baseline

Use the pinned Rust toolchain, committed lockfile, `profiling` profile, `CARGO_INCREMENTAL=0`, and
a quiescent machine. Run two independent 50-sample baselines under identical conditions:

```console
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks -- --warm-up-time 3 --measurement-time 10 --sample-size 50 --save-baseline baseline-a
CARGO_INCREMENTAL=0 cargo bench --locked --profile profiling -p serc_benchmark --bench compiler --no-default-features --features benchmarks -- --warm-up-time 3 --measurement-time 10 --sample-size 50 --save-baseline baseline-b
```

Retain every raw sample and report each benchmark's median, 95% confidence interval, Criterion
outlier classification, first-half versus second-half median drift, and cross-run median drift. Do not delete outliers or select the faster
run. More than 3% within-run or cross-run median drift, a materially wide confidence interval, an environment mismatch,
or unstable IDs makes that case inconclusive. Results from `profiling` must not be presented as
fat-LTO release results.

## CodSpeed simulation

Install `cargo-codspeed` 4.4.1 and build and run only this package and target:

```console
cargo codspeed build --locked -m simulation --profile profiling -p serc_benchmark --bench compiler --no-default-features --features "benchmarks,codspeed"
cargo codspeed run -m simulation -p serc_benchmark --bench compiler
```

CPU simulation executes one deterministic instrumented measurement per stable ID. Criterion's
local warmup and sample policy does not apply to that protocol, and simulation results must not be
compared directly with Criterion wall time. Movement within the configured CodSpeed threshold is
below the change boundary, not a confirmed optimization.

The CI workflow uses job-scoped GitHub OIDC (`contents: read`, `id-token: write`) and no static
token. CodSpeed project registration and OIDC trust for `charliermarsh/serc` must be enabled
externally before uploads can establish the first default-branch baseline; pull requests compare
against that same simulation baseline.
