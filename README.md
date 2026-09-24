# Serc

An experimental CPython bytecode compiler, written in Rust.

> [!WARNING]
>
> Serc is experimental. All code changes, PR summaries, and additional
> documentation were authored entirely by GPT-6 Astra in Codex. Use at your own
> risk.

## Highlights

- Compile Python source to CPython `.pyc` files without a CPython installation.
- Target Python 3.12, 3.13, 3.14, and 3.15 with a single executable.
- Check source files and recursively compile packages from the command line.
- Embed the compiler via the [Rust library](crates/serc).

Serc is significantly faster than CPython’s built-in bytecode compiler, while
producing byte-for-byte identical output on an extensive corpus of real-world
Python code, including [Django](https://github.com/django/django),
[Airflow](https://github.com/apache/airflow), and
[Transformers](https://github.com/huggingface/transformers).

| Project                 |              Serc | CPython 3.14.5 `compileall` | Speedup |
| ----------------------- | ----------------: | --------------------------: | ------: |
| NumPy 2.4.6 (505 files) | 1.194 s ± 0.023 s |           1.739 s ± 0.060 s |   1.46× |

However, Serc is also experimental, and so may emit incorrect bytecode in some
cases!

## Installation

Install from crates.io with Rust 1.96 or later:

```console
cargo install serc-cli --locked
```

## Getting started

Given `hello.py`:

```python
def greet(name):
    return f"Hello, {name}!"
```

Compile it to bytecode, emitted at `__pycache__/hello.cpython-314.pyc`:

```console
serc compile hello.py
```

Pass a directory to compile its Python files recursively, or use `--check` to
compile without writing cache files:

```console
serc compile path/to/package
serc compile --check path/to/package
```

Compilation uses the available CPUs by default. Use `--jobs` (or `-j`) to set a
limit, including `--jobs 1` for serial compilation. You can also set the default
with `RAYON_NUM_THREADS`:

```console
serc compile --jobs 4 path/to/package
```

### Python version

Serc emits Python 3.14-compatible bytecode by default, but supports target
selection with `--python-version`. For example, to emit Python 3.15 bytecode:

```console
serc compile --python-version 3.15 path/to/package
```

Serc supports the following CPython minor versions:

| Target | Pinned CPython release |
| ------ | ---------------------- |
| `3.12` | 3.12.14                |
| `3.13` | 3.13.15                |
| `3.14` | 3.14.5                 |
| `3.15` | 3.15.0rc2              |

Each target writes its own cache tag and bytecode magic number, so the four
versions can coexist under `__pycache__`.

## Rust library

Add Serc to your `Cargo.toml`:

```toml
[dependencies]
serc = "0.1.0"
```

Compile Python source and serialize the resulting code object in CPython's
marshal format:

```rust
let module = serc::compile("print('Hello, world!')\n", "hello.py").unwrap();
let marshaled = module.marshal();
```

Compilation defaults to Python 3.14. See the
[library guide](crates/serc#library-source-api) for selecting another Python
version and generating `.pyc` files.

## License

Serc is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in Serc by you, as defined in the Apache-2.0 license, shall be
dually licensed as above, without any additional terms or conditions.
