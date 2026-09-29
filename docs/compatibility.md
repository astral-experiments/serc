# CPython compatibility

Serc targets exact CPython releases, not arbitrary versions or inputs. Python
3.14 is the default. See the
[target pins](../crates/serc/README.md#updating-the-cpython-target) for commits
and magic numbers.

Compilation defaults to Python optimization level 0. The CLI's `-O` / `-OO`
options and the library's `CompileOptions::optimization_level` select Python's
optimization levels 1 and 2. The corpus measurements below use level 0 unless
otherwise specified.

Exact releases can still differ by native build: two CPython 3.14.5 Linux builds
fold complex NaNs to different marshal bytes. The
[native build profiles](cpython-build-profiles.md) document the reproducer,
artifact identities, explicit selection, and runtime qualification requirements.

## Required Ruff gate

From the repository root, on Linux with shared-library CPython builds:

```console
git submodule update --init vendor/ruff
./scripts/check-cpython-compatibility /path/to/python3.12.14 3.12
./scripts/check-cpython-compatibility /path/to/python3.13.15 3.13
./scripts/check-cpython-compatibility /path/to/python3.14.5
./scripts/check-cpython-compatibility /path/to/python3.15.0rc2 3.15
```

The baseline covers 2,470 `.py` fixtures from Ruff's parser, linter, formatter,
and ty semantic resources. Ruff's crates.io dependencies use release `0.0.15`;
their published source revisions and the clean submodule must all match
`0be08a206f9c3180afd3e93bcc792ed5cb1f4db1`.

| CPython   | Byte-identical accepted | CPython-rejected | Matched rejections | Serc accepted | Compiler failures |
| --------- | ----------------------: | ---------------: | -----------------: | ------------: | ----------------: |
| 3.12.14   |                   2,190 |              280 |                277 |             1 |                 2 |
| 3.13.15   |                   2,209 |              261 |                258 |             1 |                 2 |
| 3.14.5    |                   2,246 |              224 |                222 |             0 |                 2 |
| 3.15.0rc2 |                   2,266 |              204 |                202 |             0 |                 2 |

The last three columns partition CPython-rejected inputs and expose existing
rejection discrepancies. All other failure categories must be zero. Every
accepted input must match the minimal reference's marshal output byte for byte;
a compiler failure on an accepted input always fails the gate. Each outcome
count is frozen independently, so improvements also require an explicit baseline
update. Each gate builds the minimal reference with the selected interpreter and
passes source bytes to both compilers.

## Comparing another corpus

Build a minimal reference executable with the pinned CPython interpreter. This
currently requires Linux, a C compiler, and that interpreter's development
headers and shared library:

```console
/path/to/python3.14.5 scripts/build-cpython-reference.py --output /tmp/cpython-reference
cargo run --release -p serc --example compare_cpython_corpus -- \
  --python-version 3.14 --reference /tmp/cpython-reference --require-all path/to/corpus
```

Each file gets a fresh CPython process with UTF-8 mode, hash seed 0, and site
initialization disabled. The reference compiles source bytes and marshals
through the C API, without a Python wrapper or additional imports. It uses
normal interpreter startup, modeled by Serc's default intern state, and retains
the filename argument and a second reference to the root code object as a Python
caller would. The `minimal-v2` protocol fixes this ownership across Python
versions and rejects older reference artifacts.

The comparator verifies the reference protocol, CPython release, and magic
number, then compares raw marshal bytes, including string interning and object
sharing. Use `--optimize 1` or `--optimize 2` for optimized compilation;
interpreter startup stays at level 0.

`--require-all` requires at least one exact match and no failures on accepted
inputs. Add `--require-rejection-parity` to require clean Serc rejections for
every CPython-rejected input. Use `--dump-mismatches DIR` to inspect failures;
`--limit N` checks only a sorted prefix, not the complete corpus.

## Public corpus compatibility

The separately pinned [mypy_primer snapshot](../corpora/README.md) covers 73,537
`.py` and `.pyi` files. Its opt-in CPython 3.14.5 comparison supplements the
required Ruff gate:

```console
./scripts/setup-mypy-primer-corpus
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5
```

The minimal-reference baseline has 73,371 exact matches, 157 matched rejections,
one rejection mismatch, six byte mismatches, and two compiler failures. CPython
rejects 160 inputs; there are no unsupported inputs, parse/decode mismatches, or
oracle failures.

The check verifies the snapshot, builds the minimal reference with the selected
interpreter, and compares source bytes, freezing each outcome category
independently. Append `--limit 100` to check a prefix with `--require-all`
instead of the full outcome baseline. Snapshot identity and update instructions
live in the [corpus guide](../corpora/README.md).

Earlier Python-wrapper and holdout measurements are recorded in
[the measurement archive](https://github.com/astral-sh/serc/blob/1a2f5de33774fd70406a91963b2d33aea64a3aba/corpora/results/current.json).
