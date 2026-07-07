# serc

![CI](https://github.com/astral-sh/serc/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A Rust compiler from Python source to byte-for-byte-compatible CPython bytecode.

**Experimental:** This project was authored by GPT and is not intended for
public consumption. Use at your own risk.

Serc compiles Python without embedding or starting a Python runtime. The
[`serc`](crates/serc) compiler library currently targets CPython 3.14.5, and the
[`serc-cli`](crates/serc_cli) package provides the `serc` executable.

## Development

```console
SERC_TEST_PYTHON=/path/to/python3.14 cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Oracle-dependent tests fail unless `SERC_TEST_PYTHON` names the exact
CPython 3.14.5 target, including its expected bytecode magic number.

## Benchmarks

Serc has both focused Criterion/CodSpeed benchmarks and an
[end-to-end Hyperfine harness](scripts/benchmarks/README.md) that compares CPython `compileall`
with `serc compile` on a pinned NumPy source tree. See [`BENCHMARKS.md`](BENCHMARKS.md) for
published results.

## CPython compatibility

Clone the repository with submodules, then provide an exact CPython 3.14.5 executable:

```console
git clone --recurse-submodules https://github.com/astral-sh/serc
cd serc
./scripts/check-cpython-compatibility /path/to/python3.14
```

The script requires the pinned submodule corpus and compares Serc's marshal
output with `marshal.dumps(compile(...))` for 2,451 Python fixtures. It checks
the frozen split of 2,230 exact matches and 221 CPython-rejected inputs, with no
other outcomes. CI runs this gate against the exact supported CPython release
on every pull request and every push to `main`.

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

The second command freezes the current outcome baseline of 57,235 byte-identical
files, 16,143 failures, and 159 files rejected by CPython. It exits unsuccessfully
when the outcome counts change so compatibility improvements advance the baseline
explicitly. CI validates the pinned catalog and the
corpus-management scripts, but the full comparison does not replace or weaken
the Ruff compatibility gate above. See [`corpora/README.md`](corpora/README.md)
for the exact snapshot boundary and reproducibility details.

See [`crates/serc/ARCHITECTURE.md`](crates/serc/ARCHITECTURE.md) for
compiler internals and target-generation details.

## Status

Serc is experimental. It currently targets exactly CPython 3.14.5; Python
bytecode and marshal formats are not stable across Python releases. Changes to
the compiler are accepted only when the pinned compatibility corpus remains
byte-for-byte compatible with CPython. Serc was authored entirely by GPT
in Codex.

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
