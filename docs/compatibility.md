# CPython compatibility

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
[corpus comparator](../crates/serc/README.md#cpython-corpus-comparison)
for the frozen outcome counts and exact oracle requirements.

## Public corpus compatibility

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

The following public corpus measurements use CPython 3.14.5:

| Corpus | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: |
| Ruff fixtures | 2,451 | 221 | 2,230 | 0 | 100% |
| NumPy `v2.4.6` | 505 | 0 | 505 | 0 | 100% |
| mypy-primer snapshot | 73,537 | 160 | 67,850 | 5,527 | 92.47% |
| **Combined** | **76,493** | **381** | **70,585** | **5,527** | **92.74%** |

Of the 160 CPython-rejected mypy-primer files, Serc matches 67 rejections, accepts 84, and
reports a compiler failure for 9. These outcomes are frozen independently of the marshal-byte
measurements above.

The second command freezes every mypy-primer outcome category and exits unsuccessfully when any
category count changes, so compatibility improvements advance the baseline explicitly and
mismatches cannot silently turn into unsupported inputs, compiler failures, or oracle failures. CI
validates the pinned catalog and the corpus-management scripts, but the full comparison does not
replace or weaken the Ruff compatibility gate above. See
[`corpora/README.md`](../corpora/README.md) for the exact snapshot boundary and reproducibility
details.

## Holdout compatibility

To measure compatibility beyond the pinned corpora, Serc was tested against
four large OSS projects that are absent from the mypy-primer snapshot. The
July 14, 2026 holdout compiled each project's Python sources and stubs with
CPython 3.14.5 and compared the resulting marshal output byte for byte:

| Holdout project | Files | CPython-rejected | Byte-identical accepted | Byte mismatches | Accepted-input match rate |
| --- | ---: | ---: | ---: | ---: | ---: |
| [Django `bf5e73c`](https://github.com/django/django/tree/bf5e73c576c8c7d8b3ef1c03a22cf724dbb2f306) | 2,927 | 1 | 2,785 | 141 | 95.18% |
| [Airflow `cad6220`](https://github.com/apache/airflow/tree/cad6220034e87c586eb87e14b5a75f4b82984fd0) | 7,621 | 0 | 7,236 | 385 | 94.95% |
| [Transformers `ab1771c`](https://github.com/huggingface/transformers/tree/ab1771c9e42891d893189978a8009426d70b4688) | 4,643 | 0 | 4,470 | 173 | 96.27% |
| [Ansible `3217c35`](https://github.com/ansible/ansible/tree/3217c352bb7b77392317aac0cf8b859d7d43b5b5) | 1,810 | 0 | 1,659 | 151 | 91.66% |
| **Combined holdout** | **17,001** | **1** | **16,150** | **850** | **95.00%** |

The holdout had no unsupported inputs, parse/decode mismatches, compiler
failures, or oracle failures. It is an independent diagnostic measurement, not
part of the frozen compatibility baseline or the required CI gate.

See [`crates/serc/ARCHITECTURE.md`](../crates/serc/ARCHITECTURE.md) for
compiler internals and target-generation details.

## Status

Serc is experimental. Python 3.14 defaults to the exact CPython 3.14.5 target; experimental
Python 3.12, 3.13, and 3.15 support targets CPython 3.12.14, 3.13.15, and 3.15.0rc2 respectively.
Python bytecode and marshal formats are not stable across Python releases. Changes to the compiler
are accepted only when every CPython-accepted input in the required pinned corpus remains
byte-identical and the frozen outcome counts remain unchanged unless intentionally advanced. Serc was authored entirely
by GPT-6 Astra in Codex.
