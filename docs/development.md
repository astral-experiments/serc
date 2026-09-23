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
The Python style gate requires exactly Ruff 0.15.15; install it with
`python3 -m pip install ruff==0.15.15`.

`./scripts/check-ruff-pin` verifies that the Cargo manifest, lockfile, submodule gitlink, and
initialized, clean Ruff submodule all use the same revision.

## Profile-guided release build

The optional PGO build trains the actual `serc compile` command on pinned production-source
subsets of pytest, HTTPX, FastAPI, AnyIO, Zulip, Warehouse, pip, Sphinx, and astropy. Each run
copies tracked `.py` sources into a private directory, excluding tests, vendored sources,
symlinks, and stubs. Training runs serially and with up to four workers, covering discovery,
compilation, marshaling, and cache-file writes. Fetched Python is compiled, never imported or
executed. NumPy remains a held-out benchmark.

The project selection and pins follow
[Ruff's PGO recipe](https://github.com/astral-sh/ruff/blob/fb93595b28f3d38a209ed65c12fce114e249d715/scripts/build_ruff_pgo.py).
The separate training and packaging interface also follows the
[uv](https://github.com/astral-sh/uv/blob/40efebb459c21b1ecafe4787f84cb693e68a9802/scripts/build_uv_pgo.py)
and [ty](https://github.com/astral-sh/ty/blob/94ce7333abb356b2068dc98d37a68c993da6d09b/scripts/build_ty_pgo.py)
pipelines. This is a representative heuristic; performance depends on the workload and machine.
Project revisions and source roots are readable in [pgo_corpus.py](../scripts/pgo_corpus.py).

The recipe independently builds ordinary Serc with the same profile, target, and unwind
settings. Every full `.pyc` output from serial and parallel instrumented and final PGO runs
must match this control executable. It also records a per-file comparison with exact
CPython 3.14.5. At Serc `a33a6138` and the current project pins, ordinary Serc matches 559 of 564 files; five
inherited differences are retained and reported:

- `astropy/units/format/ogip.py`
- `astropy/units/quantity_helper/converters.py`
- `sphinx/ext/autodoc/_dynamic/_signatures.py`
- `sphinx/ext/autodoc/_legacy_class_based/_documenters.py`
- `warehouse/forklift/legacy.py`

The separate Ruff compatibility comparator supplies additional syntax coverage and keeps
its strict Python 3.14 gate unchanged: 2,451 files, 2,230 exact matches, zero byte mismatches,
and the existing rejection counts. It checks the exact CPython version and bytecode magic.
Every training group must produce nonempty profiles before they are merged.

Install the selected Rust toolchain's `llvm-tools-preview` component, then run:

```console
./scripts/build-pgo-release /path/to/python3.14.5
```

This uses ordinary Cargo and the repository toolchain. For local Ohm development, use
`--toolchain ohm` and set `CARGO_BUILD_BUILD_DIR` to the shared build directory used by your
other trusted worktrees. If `llvm-profdata` is not in the selected Rust sysroot, set
`LLVM_PROFDATA` to its path. Its LLVM major and minor versions must match the compiler; the
manifest records both full versions. Clear `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` first.

The script prints the final executable and manifest paths under `target/pgo-release/run-*`.
The manifest records source and artifact hashes, toolchain versions, every project pin and
source digest, per-file CPython differences, profile groups, and every command. Separate
control, generation, and use target directories are stable within each checkout;
each invocation gets fresh training data. Keep the source unchanged throughout a build and
regenerate the profile when the compiler, toolchain, or training corpus changes. Profiles and
build outputs are local artifacts and are not committed.

Use `--train-only` to stop after creating the merged profile for a later packaging build.
Use `--debug` to exercise the same complete build/train/merge/use pipeline with debug builds;
the scoped [PGO CI workflow](../.github/workflows/pgo.yml) runs this smoke check for build-pipeline
changes and retains manifests and command logs.

See the [benchmark instructions](../scripts/benchmarks/README.md#profile-guided-build) to
measure this build mode using the same workload and output preflight.

See the [compatibility guide](compatibility.md) for the frozen corpus gates and
[compiler architecture](../crates/serc/ARCHITECTURE.md) for implementation details.
