# Decompilation

`serc decompile FILE.pyc` reconstructs a whole Python module without executing
the cache or looking for the original source file. The default build provides
the compiler. Enable the optional feature when building from the workspace.

```console
cargo build --locked -p serc-cli --features decompile
target/debug/serc decompile __pycache__/example.cpython-314.pyc
```

## Workspace crates

`serc_bytecode` owns the generated target tables, marshal tags, and instruction
decoder. Its `read` feature adds cache parsing. `serc_decompile` uses those
decoded instructions for stack analysis, control-flow recovery, source
reconstruction, and verification. The public `serc::decompile_pyc` entry point
supplies serc's compiler for verification. The CLI calls that same entry point.
Compiler-only builds omit the recovery crate and the reader.

The reader recognizes timestamp, checked-hash and unchecked-hash headers for the
four pinned targets. It uses a bounded marshal arena so nested code and shared
constants can be inspected directly. The decoder resolves instruction prefixes,
inline caches, jumps and exception table entries using target metadata. A
worklist checks operand stack heights at control-flow joins and exception
handlers.

The source pass reconstructs Python from the decoded scopes. It handles common
assignments and calls, branches, loops, function and class definitions,
closures, annotations, selected generic functions, generators, comprehensions,
and formatted and template strings. Synchronous comprehensions and generator
expressions can contain multiple `for` clauses, filters, and conditional
elements. It recovers nested and starred name targets in assignments, loops,
comprehensions, and context managers. Deletion and in-place updates to
attributes, subscripts, and slices preserve evaluation order. There are
recognizers for individual exception handlers, multi-item synchronous context
managers, and sequence, mapping, and class patterns. Combinations of these
constructs can still fail.

## Verification

The decompiler tries recompiling the entire result with serc on the input target
at optimization levels 0, 1, and 2. A successful result reproduces each nested
code object's instructions, exception table, stack size, signature, flags,
names, local layout, and constants, including reference sharing. On failure the
CLI reports the reason on stderr and emits no source.

The output supplies one spelling that produces those executable fields.
Comments, original formatting, location tables, and code removed by CPython
optimization generally cannot be recovered. Traceback line numbers can change.
The decompiler preserves class line numbers when they appear as executable
constants and can refuse a module if it cannot reproduce them.

## Ruff corpus

The corpus uses the compiler's five fixture directories at Ruff revision
`9d0f58b1d2fb1cf1062d3ff99b18ebf293563f6f`. Each pinned CPython compiles the
same 2,470 files at optimization level 0. Files it rejects are excluded from
that target's denominator. For every cache serc accepts, CPython independently
recompiles the output and compares executable fields recursively. The checker
never executes corpus code.

| CPython   | Compilable modules | Verified modules | Coverage | Verified code objects / total |
| --------- | -----------------: | ---------------: | -------: | ----------------------------: |
| 3.12.14   |              2,190 |            1,537 |    70.2% |                5,785 / 11,063 |
| 3.13.15   |              2,209 |            1,495 |    67.7% |                5,085 / 11,562 |
| 3.14.5    |              2,246 |            1,490 |    66.3% |                6,244 / 14,082 |
| 3.15.0rc2 |              2,266 |            1,496 |    66.0% |                6,217 / 14,114 |

The code-object count includes each module and its nested code. Only objects
from completely verified modules count as successes. All four runs had zero
incorrect outputs, crashes, and timeouts. Current gaps include nested and
overlapping control-flow constructs, complex exception cleanup, asynchronous
comprehensions and some class and pattern layouts. A single unsupported scope
prevents recovery of the containing module.

Run the check with each pinned CPython. It prints JSON counts, grouped refusal
reasons and the filenames of any verification failures.

```console
/path/to/python3.14 -I -B scripts/check_decompile_corpus.py target/debug/serc
```
