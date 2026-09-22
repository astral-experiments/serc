# CPython compatibility

Serc targets exact CPython releases, not arbitrary versions or inputs. Python 3.14 is the
default; the 3.12, 3.13, and 3.15 targets are experimental. See the
[target pins](../crates/serc/README.md#updating-the-cpython-target) for commits and magic numbers.

## Required Ruff gate

From the repository root:

```console
git submodule update --init vendor/ruff
./scripts/check-cpython-compatibility /path/to/python3.12.14 3.12
./scripts/check-cpython-compatibility /path/to/python3.13.15 3.13
./scripts/check-cpython-compatibility /path/to/python3.14.5
./scripts/check-cpython-compatibility /path/to/python3.15.0rc2 3.15
```

The gate checks 2,451 `.py` fixtures from Ruff's parser, linter, formatter, and ty semantic
resources. Ruff's dependencies, lockfile, and clean submodule must all match revision
`e4a063a136ae0e3d6801bd0dad47d09a0f419530`. All four gates run in CI with these frozen outcomes:

| CPython | Byte-identical accepted | CPython-rejected | Matched rejections | Serc accepted | Compiler failures |
| --- | ---: | ---: | ---: | ---: | ---: |
| 3.12.14 | 2,174 | 277 | 210 | 61 | 6 |
| 3.13.15 | 2,193 | 258 | 196 | 55 | 7 |
| 3.14.5 | 2,230 | 221 | 151 | 56 | 14 |
| 3.15.0rc2 | 2,249 | 202 | 133 | 55 | 14 |

The last three columns partition CPython-rejected inputs and expose existing rejection
discrepancies. All other failure categories must be zero. Every accepted input must match
`marshal.dumps(compile(...))` byte for byte; a compiler failure on an accepted input always
fails the gate. Each outcome count is frozen independently, so improvements also require an
explicit baseline update. The 3.12, 3.13, and 3.15 gates pass source bytes to both compilers.

## Comparing another corpus

```console
cargo run --release -p serc --example compare_cpython_corpus -- \
  --python-version 3.14 --python /path/to/python3.14.5 --source-bytes --require-all path/to/corpus
```

The comparator rejects an oracle with the wrong implementation, release, or magic number, and
rejects missing or empty corpus roots. `--require-all` requires at least one exact match and no
failures on CPython-accepted inputs. Add `--require-rejection-parity` to require clean Serc
rejections for every CPython-rejected input. Use `--dump-mismatches DIR` to inspect failures;
`--limit N` checks only a sorted prefix, not the complete corpus.

## Public corpus compatibility

The separately pinned [mypy_primer snapshot](../corpora/README.md) covers 73,537 `.py` and `.pyi`
files. Its opt-in CPython 3.14.5 comparison supplements the required Ruff gate:

```console
./scripts/setup-mypy-primer-corpus
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5
```

The September 22, 2026 measurements use CPython 3.14.5 and Serc
`a5ea01b`.
Corpus revisions and membership are unchanged. The
[results summary](../corpora/results/compatibility-2026-09-22-public-a5ea01b.json) records
compiler and corpus identities, every outcome category, and evidence hashes.

| Corpus | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input compiler failures | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Ruff fixtures | 2,451 | 221 | 2,230 | 0 | 0 | 100% |
| NumPy `v2.4.6` | 505 | 0 | 505 | 0 | 0 | 100% |
| mypy-primer snapshot | 73,537 | 160 | 73,346 | 31 | 0 | 99.96% |
| **Combined** | **76,493** | **381** | **76,081** | **31** | **0** | **99.96%** |

Of the 160 CPython-rejected mypy-primer files, Serc matches 123 rejections, accepts 29, and
fails internally on 8. There are no compiler failures on accepted inputs, unsupported inputs,
parse/decode mismatches, non-UTF-8 inputs, or oracle failures. Comparisons use default intern
state and raw marshal bytes, without caller profiles, normalization, or nondeterminism exclusions.

The check verifies the snapshot before comparing source bytes and freezes each outcome category
independently. Append `--limit 100` for a prefix diagnostic without the full outcome baseline. Snapshot identity
and update instructions live in the [corpus guide](../corpora/README.md).

## Holdout compatibility

The same September 22 compiler and CPython target were tested against four OSS projects absent
from the mypy-primer snapshot. Revisions and membership are unchanged: tracked regular `.py`
files, excluding symlinks and stubs. Every CPython-accepted file matched byte for byte.

| Holdout project | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: |
| [Django `bf5e73c`](https://github.com/django/django/tree/bf5e73c576c8c7d8b3ef1c03a22cf724dbb2f306) | 2,927 | 1 | 2,926 | 0 | 100% |
| [Airflow `cad6220`](https://github.com/apache/airflow/tree/cad6220034e87c586eb87e14b5a75f4b82984fd0) | 7,621 | 0 | 7,621 | 0 | 100% |
| [Transformers `ab1771c`](https://github.com/huggingface/transformers/tree/ab1771c9e42891d893189978a8009426d70b4688) | 4,643 | 0 | 4,643 | 0 | 100% |
| [Ansible `3217c35`](https://github.com/ansible/ansible/tree/3217c352bb7b77392317aac0cf8b859d7d43b5b5) | 1,810 | 0 | 1,810 | 0 | 100% |
| **Combined holdout** | **17,001** | **1** | **17,000** | **0** | **100%** |

Serc matched the one CPython-rejected Django input. All other failure categories were zero.
This holdout comparison is a separate diagnostic, not part of the required CI gate.
