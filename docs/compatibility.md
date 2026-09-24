# CPython compatibility

Serc targets exact CPython releases, not arbitrary versions or inputs. Python 3.14 is the
default. See the
[target pins](../crates/serc/README.md#updating-the-cpython-target) for commits and magic numbers.

Exact releases can still differ by native build: two CPython 3.14.5 Linux builds fold
complex NaNs to different marshal bytes. The [build-profile diagnostic](cpython-build-profiles.md)
records the reproducer and identities; the policy for selecting these profiles is unresolved.

## Required Ruff gate

From the repository root:

```console
git submodule update --init vendor/ruff
./scripts/check-cpython-compatibility /path/to/python3.12.14 3.12
./scripts/check-cpython-compatibility /path/to/python3.13.15 3.13
./scripts/check-cpython-compatibility /path/to/python3.14.5
./scripts/check-cpython-compatibility /path/to/python3.15.0rc2 3.15
```

The baseline covers 2,470 `.py` fixtures from Ruff's parser, linter, formatter, and ty semantic
resources. Ruff's dependencies, lockfile, and clean submodule must all match revision
`62914c4b9b79a9e5004374a9c482ad2ed69290e1`.

| CPython | Byte-identical accepted | CPython-rejected | Matched rejections | Serc accepted | Compiler failures |
| --- | ---: | ---: | ---: | ---: | ---: |
| 3.12.14 | 2,190 | 280 | 269 | 9 | 2 |
| 3.13.15 | 2,209 | 261 | 256 | 3 | 2 |
| 3.14.5 | 2,246 | 224 | 219 | 3 | 2 |
| 3.15.0rc2 | 2,266 | 204 | 200 | 2 | 2 |

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

The [latest measurement](../corpora/results/current.json), from September 22, 2026, uses
CPython 3.14.5 and Serc `af3806c`. It records compiler and corpus identities, every outcome
category, and a link to the original evidence summary.

| Corpus | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input compiler failures | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Ruff fixtures | 2,451 | 221 | 2,230 | 0 | 0 | 100% |
| NumPy `v2.4.6` | 505 | 0 | 505 | 0 | 0 | 100% |
| mypy-primer snapshot | 73,537 | 160 | 73,364 | 13 | 0 | 99.98% |
| **Combined** | **76,493** | **381** | **76,099** | **13** | **0** | **99.98%** |

Of the 160 CPython-rejected mypy-primer files, Serc matches 123 rejections, accepts 29, and
fails internally on 8. There are no compiler failures on accepted inputs, unsupported inputs,
parse/decode mismatches, non-UTF-8 inputs, or oracle failures. Comparisons use default intern
state and raw marshal bytes, without caller profiles, normalization, or nondeterminism exclusions.

The check verifies the snapshot before comparing source bytes and freezes each outcome category
independently. Append `--limit 100` for a prefix diagnostic without the full outcome baseline. Snapshot identity
and update instructions live in the [corpus guide](../corpora/README.md).

## Holdout compatibility

The same compiler and CPython target were tested against four OSS projects absent from the
mypy-primer snapshot. The comparison covers tracked regular `.py` files, excluding symlinks and
stubs. Every CPython-accepted file matched byte for byte.

| Holdout project | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: |
| [Django `bf5e73c`](https://github.com/django/django/tree/bf5e73c576c8c7d8b3ef1c03a22cf724dbb2f306) | 2,927 | 1 | 2,926 | 0 | 100% |
| [Airflow `cad6220`](https://github.com/apache/airflow/tree/cad6220034e87c586eb87e14b5a75f4b82984fd0) | 7,621 | 0 | 7,621 | 0 | 100% |
| [Transformers `ab1771c`](https://github.com/huggingface/transformers/tree/ab1771c9e42891d893189978a8009426d70b4688) | 4,643 | 0 | 4,643 | 0 | 100% |
| [Ansible `3217c35`](https://github.com/ansible/ansible/tree/3217c352bb7b77392317aac0cf8b859d7d43b5b5) | 1,810 | 0 | 1,810 | 0 | 100% |
| **Combined holdout** | **17,001** | **1** | **17,000** | **0** | **100%** |

Serc matched the one CPython-rejected Django input. All other failure categories were zero.
This holdout comparison is a separate diagnostic, not part of the required CI gate.
