# Compatibility fixtures

The `fixtures` test target discovers source files at runtime. Adding a case does
not require Rust registration or rebuilding the test binary.

- `compatibility/<group>/<case>/<filename>.py` compares with CPython 3.14 at O0.
  The fixture's basename is the compiled filename.
- `optimization/<versions>/<levels>/<test>/case_NN/<filename>` selects versions
  and levels with `+`-separated lists, such as `3.12+3.13/0+1+2`. The filename
  after `case_NN` is passed to both compilers, including any subdirectories.
  Every file under this root is a source fixture; keep notes outside it.
- `private_type_parameters/<case>/private_type_parameters.py` uses the existing
  private-type-parameter oracle across all four Python targets at O0.

Source bytes, whitespace, and line endings are significant. Fixtures are
excluded from Python formatting and Git newline conversion. Keep explanations in
adjacent Markdown files, outside the compiled source.

Run all fixtures with `cargo test -p serc --test fixtures`, or append
`-- <path-substring>` to select a case or group. Generated sources and tests
with platform restrictions or additional runtime or compiler assertions remain
in Rust.
