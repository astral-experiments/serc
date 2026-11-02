# Decompilation

`serc decompile FILE.pyc` reconstructs Python source without executing the
cache or looking for the original source file. The default build provides
the compiler. Enable the optional feature when building from the workspace.

```console
cargo build --locked -p serc-cli --features decompile
target/debug/serc decompile __pycache__/example.cpython-314.pyc
```

## Workspace crates

`serc_bytecode` owns the generated target tables, marshal tags, and instruction
decoder. Its `read` feature adds cache parsing and instruction line numbers.
`serc_decompile` uses those decoded instructions for stack analysis,
control-flow recovery, source reconstruction, and verification. The public
`serc::decompile_pyc` entry point supplies serc's compiler for verification. The
CLI calls that same entry point. Compiler-only builds omit the recovery crate
and the reader.

The reader recognizes timestamp, checked-hash and unchecked-hash headers for the
four pinned targets. It uses a bounded marshal arena so nested code and shared
constants can be inspected directly. The decoder resolves instruction prefixes,
inline caches, jumps and exception table entries using target metadata. The
recovery crate builds a basic-block graph for each scope, splitting at jumps,
returns, and exception-table boundaries. It records normal predecessors and
successors separately from exceptional edges, which restore the handler's stack
depth. A worklist rejects inconsistent stack heights at joins.

Source recovery uses forward postdominators to find where branches rejoin and
shared successors to combine short-circuit tests. Loop recovery distinguishes
exhaustion from explicit breaks, including when CPython copied the return after
the loop into a break arm. It can also follow consecutive typed handlers with a
shared return and recover a conditional break through duplicated finally
cleanup. Backedges end a forward region. More complex cyclic graphs and
exception layouts can still be refused.

The source pass reconstructs ordinary and simultaneous assignments, calls,
branches, loops, imports, functions, closures, and classes. It preserves the
evaluation order of chained comparisons and parallel assignments. Class headers
can include decorators, type parameters, and metaclass keywords. Lazy type
aliases, bounds, constraints, and defaults retain their separate evaluation
scopes.

Comprehensions and generator expressions can mix synchronous and asynchronous
`for` clauses and filters. Async clauses use the exception table to find the
exhaustion block for each iterator. On 3.15 the decompiler also handles starred
elements in comprehensions. Other supported expressions include complex
literals, multidimensional slice indices, starred displays and calls, and
formatted and template strings.

Exception recovery handles selected typed and bare handlers, multi-item and
consecutive synchronous context managers, and handlers whose `try: pass` body
CPython optimized away. A generator or coroutine can suspend inside a context
manager. Recovery follows the manager's exception-table entries across an
`await` and distinguishes `with` from `async with` at each entry and exit.
Match recovery handles value and singleton patterns,
alternatives, guards, captures, and selected sequence, mapping, and class
patterns. Nested or overlapping cases can still exceed these recognizers.

## Verification

The decompiler first tries recompiling the entire result with serc on the input target
at optimization levels 0, 1, and 2. A successful result reproduces each nested
code object's instructions, exception table, stack size, signature, flags,
names, local layout, and constants, including reference sharing.

By default, if exact verification fails, serc emits best-effort source with a
`# serc: best-effort decompilation` header. When a nested body is unsupported,
an ellipsis, a comment naming the scope, and an instruction excerpt identify
the missing code. Recovery continues in the enclosing scope. A best-effort
result can differ in behavior from the input. Invalid cache headers, marshal
data, and undecodable instruction streams still produce an error.

Use `serc decompile --strict FILE.pyc` to require whole-module verification.
In strict mode a refusal writes the reason to stderr and leaves stdout empty.
Library callers can use `serc::decompile_pyc_with_mode` to inspect
`verification_error` or select `DecompileMode::Strict`.

The output supplies one spelling that produces those executable fields.
Comments, original formatting, location tables, and code removed by CPython
optimization generally cannot be recovered. Traceback line numbers can change.
The decompiler preserves class line numbers when they appear as executable
constants and can refuse a module if it cannot reproduce them.

CPython sometimes keeps a name, local binding, or constant after removing the
statement that introduced it. If ordinary recovery cannot reproduce those
fields, serc can emit a dead suite with an inline marker. For example,
`if False: hidden()` and `if False: hidden` can produce the same cache. With a
subsequent assignment to `answer`, serc emits the following source.

```console
$ serc decompile example.pyc
if False: hidden  # serc: synthetic dead code
answer = 1
```

The marker identifies the entire dead suite. A suite can contain a `yield` to
preserve a generator function or reference a name captured by a closure.

CPython can also keep a guard constant while retaining the guarded statement.
For example, on 3.14 and 3.15, compiling `if not __debug__: yield arg` with
`-O` can leave `False` in the constant table and an unconditional yield in
the instruction stream. In that case serc can emit an always-true guard.

```console
$ serc decompile optimized.pyc
def stream(arg):
    if not False: (yield arg)  # serc: synthetic live guard
    return 9
```

Here the marker applies only to the condition. The `yield` still runs. Both
markers appear by default in CLI and library output. The verifier checks the
marked spelling before accepting a whole-module match.

## Ruff corpus

The corpus uses the compiler's five fixture directories at Ruff revision
`9d0f58b1d2fb1cf1062d3ff99b18ebf293563f6f`. Each pinned CPython compiles the
same 2,470 files at optimization level 0. Files it rejects are excluded from
that target's denominator. For every cache serc accepts, CPython independently
recompiles the output and compares executable fields recursively. The checker
never executes corpus code.

| CPython   | Compilable modules | Verified modules | Coverage | Verified code objects / total |
| --------- | -----------------: | ---------------: | -------: | ----------------------------: |
| 3.12.14   |              2,190 |            1,754 |    80.1% |                7,366 / 11,063 |
| 3.13.15   |              2,209 |            1,748 |    79.1% |                7,535 / 11,562 |
| 3.14.5    |              2,246 |            1,752 |    78.0% |                9,170 / 14,082 |
| 3.15.0rc2 |              2,266 |            1,766 |    77.9% |                9,153 / 14,114 |

The code-object count includes each module and its nested code. Only objects
from completely verified modules count as successes. All four runs had zero
incorrect outputs, crashes, and timeouts. On 3.14, among the 494 refusals, 227
produced a source candidate that failed verification. The most frequent
differences were instructions, constants, and exception tables. Remaining source
recovery gaps include nested exception cleanup, overlapping branches and loops,
and patterns that combine different kinds of captures. These counts record only
the first refusal in each module.

Run the check with each pinned CPython. It prints JSON counts, first refusal
reasons and the filenames of any verification failures. To compare two builds,
pass the older executable to `--compare-with`. When the new build refuses a
file, the checker recompiles the older build's output with the same CPython. A
previously verified module in the `regressions` map fails the run even if total
coverage increased.

```console
/path/to/python3.14 -I -B scripts/check_decompile_corpus.py target/debug/serc
```
