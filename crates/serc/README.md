# Serc

Serc is an experimental Rust compiler from Python source to CPython bytecode. It defaults to
CPython 3.14.5 and has an experimental CPython 3.15.0rc2 target. It uses Ruff's parser and does not
embed or link a Python runtime. The selected CPython release is needed only to execute generated
`.pyc` files and to serve as the compatibility oracle in tests.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the pipeline, ownership boundaries, assembler contracts,
generated-target workflow, and required correctness gate.

The backend currently implements:

- CPython 3.14.5 code objects, inline caches, extended arguments, jumps, source-location tables,
  marshal references and interning, and timestamp-invalidated `.pyc` files;
- literals, collections and unpacking, f-strings, names, attributes, subscripts, slices, unary and
  binary operations, comparisons, boolean operations, conditional expressions, calls, and named
  expressions;
- assignments, deletion, imports, assertions, raises, `if`, `while`, synchronous `for`, `break`,
  `continue`, and loop `else` suites;
- functions, full parameter layouts, defaults, decorators, lambdas, globals, nonlocals, nested
  closures, deferred function annotations, and future-style module annotations;
- classes with bases, metaclass keywords, decorators, methods, class namespace metadata, class
  dictionary cells, and static-attribute metadata.

## Command-line interface

The `serc` binary accepts Python files and directories, recursively compiles directory inputs,
and writes standard timestamp-invalidated `.pyc` files under `__pycache__`:

```console
cargo run --release -p serc-cli -- compile --python-version 3.14 path/to/package
cargo run --release -p serc-cli -- compile --python-version 3.15 path/to/package
```

`--python-version` accepts `3.14` (the default, pinned to CPython 3.14.5) and `3.15` (pinned to
CPython 3.15.0rc2). Cache names use the selected version, such as `module.cpython-314.pyc` and
`module.cpython-315.pyc`, and their headers carry the corresponding magic number. Python 3.15
support remains experimental while full corpus compatibility is being established.

Paths default to the current directory. Pass `--check` to compile without writing cache files and
`--quiet` to suppress the success summary. Compilation is sequential so the command can be
compared directly with single-worker CPython `compileall`. Source files may use UTF-8 or a
CPython-compatible encoding cookie for Latin-1, Latin-9, EUC-JP, KOI8-R, or Windows-1251; other source
encodings are unsupported.

## Library source API

`compile_bytes` is the reusable file-input boundary for callers that already own source bytes. It
performs no filesystem I/O: it validates a UTF-8 BOM and encoding cookie, strictly decodes the
supported legacy encodings, rejects null bytes, and normalizes CRLF and CR newlines before
compiling. UTF-8 sources that need no normalization remain borrowed internally. For callers that
already have source metadata, `compile_bytes_to_pyc` constructs a timestamp-invalidated cache
file and records the original raw byte length:

```rust
let source = b"# coding: utf-8\r\nanswer = 42\r\n";
let pyc = serc::compile_bytes_to_pyc(source, "module.py", 1_700_000_000).unwrap();
```

Use `CompileOptions` to select a target without changing the default for other compilations:

```rust
use serc::{CompileOptions, PythonVersion, compile_bytes_with_options};

let options = CompileOptions {
    python_version: PythonVersion::Py315,
    ..CompileOptions::default()
};
let module = compile_bytes_with_options(b"answer = 42\n", "module.py", &options).unwrap();
assert_eq!(module.python_version(), PythonVersion::Py315);
let pyc = module.to_timestamp_pyc(1_700_000_000, 12);
```

`compile_with_options` accepts decoded UTF-8 source. Both options-based entry points retain the
selected target in `CompiledModule`, which controls marshal serialization and `.pyc` generation.
`CompileOptions::default()` and the existing `compile`/`compile_bytes` entry points target Python
3.14. An explicit `initial_intern_state` can be supplied in the same options value when matching
an oracle process's existing interned strings.

Filesystem-aware callers should snapshot metadata, read the source, call `compile_bytes`, then call
`CompiledModule::to_timestamp_pyc` with the snapshot. The CLI takes the snapshot before reading so
a path changed during compilation receives a header for the old snapshot and cannot validate the
new source as fresh.

## CPython corpus comparison

The `compare_cpython_corpus` example discovers every `.py` file under the vendored Ruff
submodule's linter, formatter, parser, and ty semantic test resources. It compiles every file with
both compilers. For each CPython-accepted file it compares `marshal.dumps(compile(...))` directly
with this crate's marshal output. For each CPython-rejected file it records whether Serc rejects
the input, accepts it, or fails internally:

```console
./scripts/check-cpython-compatibility /path/to/cpython-3.14.5
./scripts/check-cpython-compatibility /path/to/cpython-3.15.0rc2 3.15
```

The wrapper's optional second argument selects `3.14` (the default) or `3.15`. Each invocation
uses its exact pinned oracle and frozen outcome counts; the 3.15 gate compares source bytes.
When invoking the `compare_cpython_corpus` example directly, select the target with
`--python-version 3.15` and the oracle executable with `--python /path/to/cpython-3.15.0rc2`.
The comparator refuses any
interpreter whose implementation, version, release candidate, or magic number does not exactly
match the selected target. It also fails on a missing corpus root or an empty corpus.
Pass `--require-all` to return a failure status unless the corpus contains at least one exact match
and every CPython-accepted file matches exactly. Pass `--require-rejection-parity` to additionally
require clean Serc rejections for every CPython-rejected input. Rejection outcomes are reported
separately, and the independent CPython rejection total includes every one of them so the frozen
oracle boundary does not depend on Serc's result. The authoritative wrapper also pins the exact
and CPython-rejected totals. Additional roots can be supplied directly to the example, and
`--python`, `--limit`, and `--examples` control the oracle executable and report size. Pass
`--dump-mismatches DIR` to save mismatch inputs and outputs for inspection.

The frozen compatibility gate is pinned to CPython 3.14.5 with magic number `2b0e0d0a` and 2,451
Ruff and ty files. Its
current result is:

```text
exact:               2,230
CPython rejected:      221
matched rejection:      128
rejection mismatch:     75
byte mismatch:           0
unsupported:             0
parse/decode mismatch:   0
compiler failure:       18
non-UTF-8:               0
oracle failure:          0
```

The CPython 3.15.0rc2 gate uses the same 2,451 pinned files and requires 2,249 exact matches.
Its 202 CPython-rejected files are frozen as 132 matched rejections, 56 rejection mismatches,
and 14 compiler failures. Every other failure category must remain zero, and `--require-all`
independently rejects any failure on a CPython-accepted input. Both gates run in CI.

This exact-compatibility result is specific to the pinned oracle version and frozen corpus; it is
not a claim about arbitrary CPython versions or Python inputs. The exact-output gate remains fully
green; the separately reported rejection outcomes expose existing validation debt instead of
hiding it behind the oracle's rejection.

## Updating the CPython target

Opcode IDs, inline-cache widths, opcode flags and predicates, operand values, stack metadata, code
flags, local-kind bits, magic number, marshal format tags, and target provenance are generated from
these exact CPython releases:

| Target | CPython tag | Commit | Magic number |
| --- | --- | --- | --- |
| `3.14` (default) | `v3.14.5` | `5607950ef232dad16d75c0cf53101d9649d89115` | `2b0e0d0a` |
| `3.15` (experimental) | `v3.15.0rc2` | `435c9e5a798c99653e3ab64ce29baed0e4f3dfee` | `520e0d0a` |

Regenerate each checked-in module from its pinned checkout, then verify that it is current:

```console
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.14 --target-version 3.14
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.14 --target-version 3.14 --check
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.15 --target-version 3.15
python3 crates/serc/scripts/generate_cpython_target.py --cpython /path/to/cpython-3.15 --target-version 3.15 --check
```

The generator defaults to target `3.14`, rejects a checkout at any other commit for the selected
target, and reads every input through that pinned commit, so uncommitted CPython changes cannot be mislabeled as generated target data. It records
hashes for every CPython source file that contributes target or marshal-format metadata. Instruction
metadata is emitted under `src/target`; the independently versioned marshal protocol constants are
emitted under `src/marshal`.

The Ruff dependencies are pinned to a specific Git revision because Ruff's parser crates are not
published independently. From the workspace root, `./scripts/check-ruff-pin` verifies that the
Cargo manifest, lockfile, submodule gitlink, and initialized, clean submodule all select the same
revision.

From the workspace root, `./scripts/check-cpython-compatibility /path/to/python3.14.5` and
`./scripts/check-cpython-compatibility /path/to/python3.15.0rc2 3.15` require the initialized matching
Ruff submodule and run their complete frozen gates without mutating the checkout.

## Benchmarks

The [Serc benchmark suite](../serc_benchmark/README.md) measures the public compile and
marshal boundaries over fixed, CPython-validated workloads. Its documentation records the stable
benchmark identities and the canonical local Criterion and CodSpeed simulation commands.

The corpus benchmark compares Serc and CPython within one long-lived process per engine. Run
each phase separately from the workspace root; for example:

```console
cargo run --locked --profile profiling -p serc --example benchmark_cpython_corpus -- \
  --python /path/to/python3.14.5 \
  --phase compiler_core \
  --warmups 5 \
  --samples 21 \
  --cooldown-ms 5000 \
  --output target/serc-compiler-core.tsv \
  vendor/ruff/crates/ty_python_semantic/resources/corpus \
  vendor/ruff/crates/ruff_python_parser/resources/valid \
  vendor/ruff/crates/ruff_python_parser/resources/inline/ok \
  vendor/ruff/crates/ruff_linter/resources/test/fixtures \
  vendor/ruff/crates/ruff_python_formatter/resources/test/fixtures
```

Repeat with `compile_marshal` and `marshal_only` to measure the other boundaries. Before any
warmup or measured block, the harness compiles the accepted corpus with both engines and compares
every marshal payload byte-for-byte. That preflight adds startup work and temporarily retains the
CPython payloads, but it is outside all timed samples; the existing `compiler_core`,
`compile_marshal`, and `marshal_only` timing boundaries are unchanged. The TSV records the actual
Cargo profile, full Serc commit and dirty-diff hash, lockfile and source/filename corpus hashes,
and the resolved benchmark and Python executable identities.

The [end-to-end Hyperfine harness](../../scripts/benchmarks/README.md) compares CPython
`compileall` with the `serc compile` workflow on a pinned NumPy checkout, including process
startup, source discovery, marshaling, and `.pyc` writes.
