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

From the repository root:

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
| 3.12.14   |                   2,190 |              280 |                271 |             7 |                 2 |
| 3.13.15   |                   2,209 |              261 |                258 |             1 |                 2 |
| 3.14.5    |                   2,246 |              224 |                221 |             1 |                 2 |
| 3.15.0rc2 |                   2,266 |              204 |                202 |             0 |                 2 |

The last three columns partition CPython-rejected inputs and expose existing
rejection discrepancies. All other failure categories must be zero. Every
accepted input must match `marshal.dumps(compile(...))` byte for byte; a
compiler failure on an accepted input always fails the gate. Each outcome count
is frozen independently, so improvements also require an explicit baseline
update. The 3.12, 3.13, and 3.15 gates pass source bytes to both compilers.

## Comparing another corpus

Build a minimal reference executable with the pinned CPython interpreter. This
currently requires Linux, a C compiler, and that interpreter's development
headers and shared library:

```console
/path/to/python3.14.5 scripts/build-cpython-reference.py --output /tmp/cpython-reference
cargo run --release -p serc --example compare_cpython_corpus -- \
  --python-version 3.14 --reference /tmp/cpython-reference --require-all path/to/corpus
```

Each file gets a fresh, isolated CPython process with UTF-8 mode, a fixed hash
seed, and site initialization disabled. The C reference reads the source bytes
and calls CPython's compiler and marshal writer directly. It runs no Python
wrapper and imports no additional modules. Normal interpreter initialization
still supplies the startup state modeled by Serc's default target metadata. It
retains a second reference to the root code object while marshaling, matching a
named `code` variable passed to `marshal.dumps(code)`.

The comparator validates the reference protocol, CPython release, and magic
number before comparing raw marshal bytes. String interning, object sharing,
reference indexes, and all code-object fields remain part of byte equality. Use
`--optimize 1` or `--optimize 2` to select the compilation optimization level;
interpreter startup always uses level 0. The CLI uses the same default intern
state as the library.

`--require-all` requires at least one exact match and no failures on accepted
inputs. Add `--require-rejection-parity` to require clean Serc rejections for
every CPython-rejected input. Use `--dump-mismatches DIR` to inspect failures;
`--limit N` checks only a sorted prefix, not the complete corpus.

### Existing Python-script comparisons

The `--python /path/to/python3.14.5 --source-bytes` mode retains the Python
wrapper used by the recorded corpus measurements below. The comparator rejects
an oracle with the wrong implementation, release, or magic number, and rejects
missing or empty corpus roots. Without an explicit caller-state profile, this is
a diagnostic comparison using Serc's isolated startup state. The oracle imports
`pathlib`, which can intern additional strings and change marshal sharing. A
byte mismatch alone therefore does not establish a compiler regression. Exact
counts still report observed byte equality.

In this mode, `--require-all` requires a validated
[initial intern-state profile](../crates/serc/examples/support/initial-intern-profile.md)
for the oracle caller, at least one exact match, and no failures on
CPython-accepted inputs. Python 3.12 additionally requires
`--caller-intern-model MODEL.json`, since `pathlib` interns the current
filename's components; one fixed snapshot cannot describe different input paths.
Profiles and models retain their documented assumptions: a captured process
snapshot does not prove the absence or lifetime of every other interned string.
The explicit `--caller-intern-assume-absent` override remains diagnostic.

For an observed-byte regression check under startup assumptions, use
`--require-all --diagnostic-startup-state`. The explicit diagnostic override
preserves the accepted-input failure checks but does not establish that the
caller's state matches. The Ruff baseline and limited mypy-primer checks use
this mode; their expected counts and failure categories are unchanged. Caller
profiles and native output caches cannot be combined with `--reference`.

## Public corpus compatibility

The separately pinned [mypy_primer snapshot](../corpora/README.md) covers 73,537
`.py` and `.pyi` files. Its opt-in CPython 3.14.5 comparison supplements the
required Ruff gate:

```console
./scripts/setup-mypy-primer-corpus
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5
```

The [latest measurement](../corpora/results/current.json), from September 22,
2026, uses CPython 3.14.5 and Serc `af3806c`. It records compiler and corpus
identities, every outcome category, and a link to the original evidence summary.
The Ruff fixture measurement uses commit
`e4a063a136ae0e3d6801bd0dad47d09a0f419530`, earlier than the required gate.

| Corpus               |      Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input compiler failures | Accepted-input match rate |
| -------------------- | ---------: | ---------------: | ----------------------: | --------------: | -------------------------------: | ------------------------: |
| Ruff fixtures        |      2,451 |              221 |                   2,230 |               0 |                                0 |                      100% |
| NumPy `v2.4.6`       |        505 |                0 |                     505 |               0 |                                0 |                      100% |
| mypy-primer snapshot |     73,537 |              160 |                  73,364 |              13 |                                0 |                    99.98% |
| **Combined**         | **76,493** |          **381** |              **76,099** |          **13** |                            **0** |                **99.98%** |

Of the 160 CPython-rejected mypy-primer files, Serc matches 123 rejections,
accepts 29, and fails internally on 8. There are no compiler failures on
accepted inputs, unsupported inputs, parse/decode mismatches, non-UTF-8 inputs,
or oracle failures. Comparisons use default intern state and raw marshal bytes,
without caller profiles, normalization, or nondeterminism exclusions.

The check verifies the snapshot before comparing source bytes and freezes each
outcome category independently. Append `--limit 100` for a prefix diagnostic
without the full outcome baseline. Snapshot identity and update instructions
live in the [corpus guide](../corpora/README.md).

## Holdout compatibility

The same compiler and CPython target were tested against four OSS projects
absent from the mypy-primer snapshot. The comparison covers tracked regular
`.py` files, excluding symlinks and stubs. Every CPython-accepted file matched
byte for byte.

| Holdout project                                                                                                     |      Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input match rate |
| ------------------------------------------------------------------------------------------------------------------- | ---------: | ---------------: | ----------------------: | --------------: | ------------------------: |
| [Django `bf5e73c`](https://github.com/django/django/tree/bf5e73c576c8c7d8b3ef1c03a22cf724dbb2f306)                  |      2,927 |                1 |                   2,926 |               0 |                      100% |
| [Airflow `cad6220`](https://github.com/apache/airflow/tree/cad6220034e87c586eb87e14b5a75f4b82984fd0)                |      7,621 |                0 |                   7,621 |               0 |                      100% |
| [Transformers `ab1771c`](https://github.com/huggingface/transformers/tree/ab1771c9e42891d893189978a8009426d70b4688) |      4,643 |                0 |                   4,643 |               0 |                      100% |
| [Ansible `3217c35`](https://github.com/ansible/ansible/tree/3217c352bb7b77392317aac0cf8b859d7d43b5b5)               |      1,810 |                0 |                   1,810 |               0 |                      100% |
| **Combined holdout**                                                                                                | **17,001** |            **1** |              **17,000** |           **0** |                  **100%** |

Serc matched the one CPython-rejected Django input. All other failure categories
were zero. This holdout comparison is a separate diagnostic, not part of the
required CI gate.
