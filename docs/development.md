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
its exact release and bytecode magic number. `SERC_TEST_PYTHON` remains the Python 3.14 oracle;
the other versions use their suffixed variables. CI provisions all four interpreters on Linux,
macOS, and Windows and checks generated metadata against each pinned CPython source commit.
Python 3.12.14 is built from its pinned source on macOS and Windows, where `setup-python` does
not provide this security release.
The Python style gate requires exactly Ruff 0.15.15; install it with
`python3 -m pip install ruff==0.15.15`.

See the [compatibility guide](compatibility.md) for the frozen corpus gates and
[compiler architecture](../crates/serc/ARCHITECTURE.md) for implementation details.
