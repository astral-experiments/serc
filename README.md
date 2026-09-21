# serc

![CI](https://github.com/astral-sh/serc/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A Rust compiler for Python source that targets byte-for-byte compatibility with CPython 3.14.5 on
pinned corpora, with experimental CPython 3.12.14, 3.13.15, and 3.15.0rc2 support.

**Experimental:** This project was authored by GPT and is not intended for
public consumption. Use at your own risk.

Serc compiles Python without embedding or starting a Python runtime. The
[`serc`](crates/serc) compiler library defaults to CPython 3.14.5, and the
[`serc-cli`](crates/serc_cli) package provides the `serc` executable.

## Python version

Choose a target for each compilation with `--python-version`:

```console
serc compile --python-version 3.12 path/to/package
serc compile --python-version 3.13 path/to/package
serc compile --python-version 3.14 path/to/package
serc compile --python-version 3.15 path/to/package
```

Library callers select `PythonVersion::Py312`, `Py313`, `Py314`, or `Py315` through
`CompileOptions`, then call `compile_with_options` or `compile_bytes_with_options`. See the
[library API example](crates/serc/README.md#library-source-api).

## Development

```console
cargo fmt --all --check
./scripts/check-python-style
SERC_TEST_PYTHON_312=/path/to/python3.12.14 \
SERC_TEST_PYTHON_313=/path/to/python3.13.15 \
SERC_TEST_PYTHON=/path/to/python3.14.5 \
SERC_TEST_PYTHON_315=/path/to/python3.15.0rc2 \
  cargo test --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Oracle-dependent tests use the four executable paths above. Each interpreter is checked against
its exact release and bytecode magic number. `SERC_TEST_PYTHON` remains the Python 3.14 oracle;
the other versions use their suffixed variables. CI provisions all four interpreters on Linux,
macOS, and Windows and checks generated metadata against each pinned CPython source commit.
Python 3.12.14 is built from its pinned source on macOS and Windows, where `setup-python` does
not provide this security release.
The Python style gate requires exactly Ruff 0.15.15; install it with
`python3 -m pip install ruff==0.15.15`.

## Benchmarks

Serc has both focused Criterion/CodSpeed benchmarks and an
[end-to-end Hyperfine harness](scripts/benchmarks/README.md) that compares CPython `compileall`
with `serc compile` on a pinned NumPy source tree. See [`BENCHMARKS.md`](BENCHMARKS.md) for
published results.

The latest published end-to-end measurement (September 20, 2026, Serc
[`1b57c9c`](https://github.com/astral-sh/serc/commit/1b57c9c33f7586a85d8a7ae2a5f3dd3b9e6a5cec))
compiled NumPy's 505 Python files in **1.194 s ± 0.023 s**, compared with
**1.739 s ± 0.060 s** for CPython 3.14.5 `compileall`: **1.46× faster** on
that machine and workload. These are means and standard deviations across 50
runs per compiler; all 505 generated `.pyc` files matched byte for byte.

## CPython compatibility

Clone the repository with submodules, then run the frozen gates for each target:

```console
git clone --recurse-submodules https://github.com/astral-sh/serc
cd serc
./scripts/check-cpython-compatibility /path/to/python3.12.14 3.12
./scripts/check-cpython-compatibility /path/to/python3.13.15 3.13
./scripts/check-cpython-compatibility /path/to/python3.14.5
./scripts/check-cpython-compatibility /path/to/python3.15.0rc2 3.15
```

The script requires the pinned submodule corpus and runs the exact CPython
oracle over 2,451 Python fixtures. The Python 3.14 gate accepts 2,230 inputs and
requires Serc's marshal output to match `marshal.dumps(compile(...))`
exactly. The frozen baseline separately records 221 CPython-rejected inputs: 151 matched Serc
rejections, 56 inputs accepted by Serc, and 14 internal compiler failures. CI freezes each
outcome category against the exact supported CPython release on every pull request and every push
to `main`.

The optional second argument selects `3.12`, `3.13`, `3.14` (the default), or `3.15`. The Python 3.15 gate
uses the same 2,451 files and passes source bytes to both compilers. It requires all 2,249
CPython-accepted files to match exactly, and freezes the 202 CPython-rejected inputs as
133 matched rejections, 55 rejection mismatches, and 14 compiler failures. Compiler failures
on accepted inputs always fail any gate.

The Python 3.12.14 gate requires 2,174 exact matches and records 277 CPython-rejected inputs;
Python 3.13.15 requires 2,193 exact matches and records 258 CPython-rejected inputs. Both use
the same 2,451 fixtures and pass source bytes to both compilers. See the
[corpus comparator](crates/serc/README.md#cpython-corpus-comparison)
for the frozen outcome counts and exact oracle requirements.

### Public corpus compatibility

For broader diagnostics, Serc also has a separately pinned corpus derived
from [`mypy_primer`](https://github.com/hauntsaninja/mypy_primer). The frozen
snapshot contains 73,537 tracked `.py` and `.pyi` files from 162 logical catalog
entries across 160 repositories. It is intentionally opt-in because
materializing and checking its 691,638,412 source bytes requires substantial
network, storage, and CPU resources:

```console
./scripts/setup-mypy-primer-corpus
./scripts/check-mypy-primer-compatibility /path/to/python3.14.5
```

The September 21, 2026 measurements below use CPython 3.14.5 and Serc
[`671f35c`](https://github.com/astral-sh/serc/commit/671f35cade80d4bab3e41f111b69e31077bdc483).
The corpus revisions and membership are unchanged. Counts and measurement details are recorded
in the [results summary](corpora/results/compatibility-2026-09-21.json).

| Corpus | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input compiler failures | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Ruff fixtures | 2,451 | 221 | 2,230 | 0 | 0 | 100% |
| NumPy `v2.4.6` | 505 | 0 | 505 | 0 | 0 | 100% |
| mypy-primer snapshot | 73,537 | 160 | 73,097 | 278 | 2 | 99.62% |
| **Combined** | **76,493** | **381** | **75,832** | **278** | **2** | **99.63%** |

Of the 160 CPython-rejected mypy-primer files, Serc matches 123 rejections, accepts 29, and
reports a compiler failure for 8. These outcomes are frozen independently of the marshal-byte
measurements above. The two additional compiler failures on CPython-accepted inputs count
against the match rate. There are no unsupported inputs, parse/decode mismatches, non-UTF-8
inputs, or oracle failures.

The second command freezes every mypy-primer outcome category and exits unsuccessfully when any
category count changes, so compatibility improvements advance the baseline explicitly and
mismatches cannot silently turn into unsupported inputs, compiler failures, or oracle failures. CI
validates the pinned catalog and the corpus-management scripts, but the full comparison does not
replace or weaken the Ruff compatibility gate above. See
[`corpora/README.md`](corpora/README.md) for the exact snapshot boundary and reproducibility
details.

### Holdout compatibility

To measure compatibility beyond the pinned corpora, Serc was tested against
four large OSS projects that are absent from the mypy-primer snapshot. The
September 21, 2026 rerun uses the same compiler and CPython target as the public measurements
above, comparing marshal output byte for byte. The published source revisions and membership
are unchanged: tracked regular `.py` files, excluding symlinks and stubs. This preserves the
published `.py`-only file counts and corrects the earlier description that mentioned stubs.

| Holdout project | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: |
| [Django `bf5e73c`](https://github.com/django/django/tree/bf5e73c576c8c7d8b3ef1c03a22cf724dbb2f306) | 2,927 | 1 | 2,921 | 5 | 99.83% |
| [Airflow `cad6220`](https://github.com/apache/airflow/tree/cad6220034e87c586eb87e14b5a75f4b82984fd0) | 7,621 | 0 | 7,610 | 11 | 99.86% |
| [Transformers `ab1771c`](https://github.com/huggingface/transformers/tree/ab1771c9e42891d893189978a8009426d70b4688) | 4,643 | 0 | 4,639 | 4 | 99.91% |
| [Ansible `3217c35`](https://github.com/ansible/ansible/tree/3217c352bb7b77392317aac0cf8b859d7d43b5b5) | 1,810 | 0 | 1,795 | 15 | 99.17% |
| **Combined holdout** | **17,001** | **1** | **16,965** | **35** | **99.79%** |

The holdout had no unsupported inputs, parse/decode mismatches, compiler
failures, or oracle failures. It is an independent diagnostic measurement, not
part of the frozen compatibility baseline or the required CI gate.

See [`crates/serc/ARCHITECTURE.md`](crates/serc/ARCHITECTURE.md) for
compiler internals and target-generation details.

## Status

Serc is experimental. Python 3.14 defaults to the exact CPython 3.14.5 target; experimental
Python 3.12, 3.13, and 3.15 support targets CPython 3.12.14, 3.13.15, and 3.15.0rc2 respectively.
Python bytecode and marshal formats are not stable across Python releases. Changes to the compiler
are accepted only when every CPython-accepted input in the required pinned corpus remains
byte-identical and the frozen outcome counts remain unchanged unless intentionally advanced. Serc was authored entirely
by GPT in Codex.

## License

serc is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in serc by you, as defined in the Apache-2.0
license, shall be dually licensed as above, without any additional terms or
conditions.
