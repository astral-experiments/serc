- Preserve byte-for-byte compatibility with the pinned CPython target.
- Run `cargo fmt --all --check`, strict workspace Clippy, and all-target tests for source changes.
- Run `./scripts/check-cpython-compatibility /path/to/cpython-3.14.5` for compiler, assembler,
  marshal, target, or dependency changes.
- Keep Ruff and the compatibility corpus pinned to the same revision.
- Keep target metadata generated from the exact CPython tag and commit documented by the crate.
- Do not weaken marshal identity, table ordering, assembler pass ordering, or oracle identity checks.
