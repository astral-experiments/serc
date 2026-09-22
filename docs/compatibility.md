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
files. Its opt-in CPython 3.14.5 comparison currently freezes 67,850 byte-identical outputs and
5,527 byte mismatches: **92.47% of 73,377 CPython-accepted inputs**. Of 160 CPython-rejected
files, Serc rejects 67, accepts 84, and fails internally on 9. Every other failure category is
zero.

```console
./scripts/setup-mypy-primer-corpus
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5
```

The check verifies the snapshot before comparing source bytes and freezes each outcome category
independently. Append `--limit 100` for a prefix diagnostic without the full outcome baseline. Snapshot identity and update instructions live in the
[corpus guide](../corpora/README.md).
