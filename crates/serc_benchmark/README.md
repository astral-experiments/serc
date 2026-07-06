# Serc benchmarks

This crate provides stable performance baselines for Serc's public compile and marshal seams.
It is a non-published harness crate; it does not add a benchmark-only API to the compiler.

## Workloads and provenance

The suite embeds 23 fixed sources in four workload sets. Every source and logical filename is
validated byte-for-byte against CPython tag `v3.14.5`, commit
`5607950ef232dad16d75c0cf53101d9649d89115`, by Serc's compatibility tests.

### Compile microbenchmarks

The 16 focused cases isolate related syntax families while still including enough variants to
exercise their lowering and assembler paths.

| Stable case ID | Bytes | Focus |
| --- | ---: | --- |
| `micro_baseline` | 5 | fixed module, parser, and compiler overhead |
| `micro_literals` | 487 | scalar, collection, formatted-string, and template literals |
| `micro_operations` | 507 | arithmetic, bitwise, boolean, comparison, and conditional expressions |
| `micro_calls` | 364 | calls, arguments, attributes, subscripts, slices, stores, and deletes |
| `micro_functions` | 375 | parameters, defaults, decorators, lambdas, closures, and `nonlocal` |
| `micro_classes` | 398 | class bodies, decorators, metaclasses, and method decorators |
| `micro_branches` | 321 | `if`, `elif`, nesting, and boolean branches |
| `micro_loops` | 401 | `for`, `while`, `break`, and `continue` |
| `micro_exceptions` | 434 | `try`, handlers, `else`, `finally`, `raise`, and `except*` |
| `micro_with` | 188 | single and multiple context managers |
| `micro_comprehensions` | 504 | list, dict, set, generator, nested, and named comprehensions |
| `micro_generators` | 236 | `yield`, `yield from`, generator expressions, and generator lambdas |
| `micro_pattern_matching` | 534 | literal, OR, sequence, wildcard-star, and capture patterns |
| `micro_async` | 426 | coroutines, `await`, async contexts, iteration, and comprehensions |
| `micro_annotations` | 350 | annotations, type aliases, type parameters, defaults, and bounds |
| `micro_imports` | 326 | absolute, relative, aliased, star, and local imports |

### Composite workloads

The original four construct-rich workloads remain stable interaction benchmarks for both compile
and marshal measurements.

| Stable case ID | Logical filename | Bytes | Construct coverage |
| --- | --- | ---: | --- |
| `context_managers` | `benchmark/context_managers.py` | 775 | classes, nested contexts, suppression, loop exits |
| `annotation_scopes` | `annotation_scopes.py` | 479 | nested code, generics, aliases, annotation scopes |
| `match_control_flow` | `match.py` | 1,169 | match cases and guards, `except*`, comprehension |
| `loop_async_edges` | `loop_tail.py` | 2,019 | sync/async loops, cleanup, generators, boolean tails |

### Large workload

`large_dataset` is a 42,660-byte real-world Python test module. It is copied byte-for-byte from
Ruff's pinned `large/dataset.py` benchmark resource and preserves Ruff's upstream MikeIO
provenance. This follows Ruff's practice of checking representative real-world sources into its
benchmark crate so runs remain offline and reproducible. See the
[fixture provenance](../serc/tests/fixtures/benchmark/README.md).

### Marshal microbenchmarks

Marshal uses a separate focused set rather than automatically serializing every syntax microcase.

| Stable case ID | Bytes | Output shape |
| --- | ---: | --- |
| `constants` | 489 | repeated strings, scalar widths, bytes, nested tuples, and frozen sets |
| `nested_code` | 570 | closures, nested functions and classes, lambdas, comprehensions, and generators |

A source-byte or logical-filename change is semantic and requires a new benchmark ID and baseline
rather than silently continuing an existing series. New cases receive new IDs without changing
the existing identities. Logical filenames are intentionally stable because they are serialized
into code objects.

## Measurement boundary

The suite reports 28 stable measurements. The `compile` group contains the 16 microbenchmarks,
four composite workloads, and large workload. The `marshal` group contains the two marshal
microbenchmarks, four composite workloads, and large workload.

`compile` times one `serc::compile` call, including normalization, Ruff parsing, lowering, all
assembler passes, layout, and module construction. `marshal` compiles once outside the timed region
and then times one fresh `CompiledModule::marshal` call. Both groups report source bytes as
throughput. Both use explicit `BatchSize::LargeInput` batching: the setup closure is empty, output
destruction remains outside the timed region, and retained heap-owning outputs are bounded by
Criterion's large-value policy.

Each case is compiled successfully once before measurement. Complete marshal-byte equality is
checked separately by Serc's ordinary compatibility test and the frozen CPython corpus gate;
CPython invocation, fixture discovery, assertions, and output hashing are never timed.

There is deliberately no isolated assembler benchmark. Serc has no stable public prepared-IR
seam, and exposing one would expand the compiler's compatibility surface. There are also no
compile-plus-marshal, batch, parser-only, allocator, memory, or walltime series in this baseline.

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
