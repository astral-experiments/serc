# Development

Run the following commands from a checkout cloned with `--recurse-submodules`:

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
its exact release and bytecode magic number. CI also checks generated target metadata.
The Python style gate requires exactly Ruff 0.16.8; install it with
`python3 -m pip install ruff==0.16.8`.

`./scripts/check-ruff-pin` verifies that the Cargo manifest, lockfile, submodule gitlink, and
initialized, clean Ruff submodule all use the same revision.

## Profile-guided release build

Install the Rust toolchain's `llvm-tools-preview` component, then run:

```console
./scripts/build-pgo-release /path/to/python3.14.5
```

The recipe trains `serc compile` serially and with up to four workers on nine
[pinned projects](../scripts/pgo_corpus.py), following
[Ruff's project selection](https://github.com/astral-sh/ruff/blob/fb93595b28f3d38a209ed65c12fce114e249d715/scripts/build_ruff_pgo.py).
It compiles tracked `.py` sources without importing or executing them, excluding tests,
vendored code, symlinks, and stubs. NumPy is held out for benchmarking.

Instrumented and final PGO builds must produce the same complete `.pyc` files as an
independently built ordinary executable. The manifest also records per-file differences
against CPython 3.14.5; those files remain in training. The separate pinned Ruff
compatibility gate remains strict and supplies additional syntax coverage.

The executable, merged profile, and manifest are written under `target/pgo-release/run-*`.
The manifest records commands, toolchains, project pins, and source/artifact hashes.
Keep the source unchanged during a build and regenerate profiles after changing the
source, toolchain, or corpus.

Use `--train-only` to generate a profile for a later packaging build. Use `--debug` to run
the full pipeline with debug builds, as the [PGO CI workflow](../.github/workflows/pgo.yml) does.

Clear `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` before running. Use `--toolchain <name>`
to select another Rustup toolchain. If `llvm-profdata` is outside its sysroot, set
`LLVM_PROFDATA` to its path; its LLVM major and minor versions must match the compiler.

See the [benchmark instructions](../scripts/benchmarks/README.md#profile-guided-build)
to measure the resulting profile.

See the [compatibility guide](compatibility.md) for the frozen corpus gates and
[compiler architecture](../crates/serc/ARCHITECTURE.md) for implementation details.
