# Serc

Serc is a Rust workspace for compiling Python source into byte-for-byte-compatible CPython
bytecode without embedding or starting a Python runtime.

The [`serc`](crates/serc) crate currently targets CPython 3.14.5 and uses Ruff's parser. Its
compiler history was extracted from the original `uv` workspace without squashing the individual
implementation, compatibility, optimization, and maintenance commits.

## Development

```console
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

## CPython compatibility

Clone the repository with submodules, then provide an exact CPython 3.14.5 executable:

```console
git clone --recurse-submodules https://github.com/astral-sh/serc
cd serc
./scripts/check-cpython-compatibility /path/to/python3.14
```

The script initializes the pinned Ruff corpus when needed and compares Serc's marshal output
with `marshal.dumps(compile(...))` for 2,451 Ruff and ty fixtures. Every CPython-accepted file must
match byte-for-byte.

See [`crates/serc/ARCHITECTURE.md`](crates/serc/ARCHITECTURE.md) for compiler internals and
target-generation details.
