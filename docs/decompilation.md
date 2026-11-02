# Decompilation

`serc decompile FILE.pyc` reconstructs Python source without executing the cache
or looking for the original source file. The default build provides the
compiler. Enable the optional feature when building from the workspace.

```console
cargo build --locked -p serc-cli --features decompile
target/debug/serc decompile __pycache__/example.cpython-314.pyc
```

## Workspace crates

`serc_bytecode` owns the generated target tables, marshal tags, and instruction
decoder. Its `read` feature adds cache parsing and instruction line numbers.
`serc_decompile` uses those decoded instructions for stack analysis,
control-flow recovery, source reconstruction, and verification. The public
`serc::decompile_pyc` and `serc::decompile_pyc_with_mode` entry points supply
serc's compiler for verification. The CLI selects the mode through the same API.
Compiler-only builds omit the recovery crate and the reader.

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
`await` and distinguishes `with` from `async with` at each entry and exit. Match
recovery handles value and singleton patterns, alternatives, guards, captures,
and selected sequence, mapping, and class patterns. Nested or overlapping cases
can still exceed these recognizers.

CPython can build long dictionary displays one pair at a time. Recognizing those
insertions lets recovery continue past the configuration dictionary in 3.14.5's
`turtle.py`, where 182 of 250 code objects then match. In `codecs.py`, the
module begins with a handler that binds an exception and always raises. The
exception table separates the handler body from CPython's cleanup of the bound
name. Following its normal continuation recovers 82 of 100 code objects.

In 3.14.5's `importlib._bootstrap_external`, the module starts with
`assert all(len(sep) == 1 for sep in path_separators)`. CPython emits two paths,
an inlined loop for the builtin and an ordinary call for a replacement bound to
`all`. Recovering that call and the formatted set comprehension that follows it
places the later definitions back in the module. CPython reproduces 92 of its
125 code objects.

## Verification

The decompiler first tries recompiling the entire result with serc on the input
target at optimization levels 0, 1, and 2. A successful result reproduces each
nested code object's instructions, exception table, stack size, signature,
flags, names, local layout, and constants, including reference sharing.

By default, if exact verification fails, serc emits best-effort source with a
`# serc: best-effort decompilation` header. When a nested body is unsupported,
an ellipsis and a comment naming the scope identify the missing code. Recovery
continues at a statement boundary once the evaluation stack is empty and the
skipped paths rejoin, or in the enclosing scope. A best-effort result can differ
in behavior from the input. Invalid cache headers, marshal data, and undecodable
instruction streams still produce an error.

If the definition site is lost but a named function's code object survives, serc
appends its body as comments. Each entry starts with
`# serc: detached function`, the original qualified name, and the original line
number. Parameters and coroutine flags come from the code object. The enclosing
code supplies decorators, annotations, and default values, so detached
signatures omit those details. A nested definition recovered inside a detached
body appears with its parent. These comments never bind a name or run a
decorator. The appendix covers named functions and omits anonymous and
compiler-generated scopes. Recovery stops at the shared work or output limit.

After any detached functions, serc lists the original basic blocks covering each
gap, including gaps in detached bodies. Each block appears once per code object.
A header gives its starting byte offset, fallthrough and jump targets, and any
exception handler with its unwind depth and `lasti` flag. Instruction lines show
the byte offset, opcode, and numeric argument. Offsets are local to the code
object. The listing includes the whole block when recovery stops mid-block, and
can include earlier blocks for context when an incomplete suite is discarded. A
lost class or anonymous function referenced by the skipped instructions can have
its own listing. The appendix has a separate 16 MiB output budget. Reaching it
leaves a truncation comment in the output.

Use `serc decompile --strict FILE.pyc` to require whole-module verification. In
strict mode a refusal writes the reason to stderr and leaves stdout empty.
Library callers can use `serc::decompile_pyc_with_mode` to inspect
`verification_error` or select `DecompileMode::Strict`.

Verified output supplies one spelling that produces those executable fields.
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
preserve a generator function or an assignment to bind a closure cell. For
example, a nested `nonlocal x` can outlive the outer function's erased
annotation `x: int`. A marked dead assignment binds the name while leaving the
cell empty at runtime.

CPython can also keep a guard constant while retaining the guarded statement.
For example, on 3.14 and 3.15, compiling `if not __debug__: yield arg` with `-O`
can leave `False` in the constant table and an unconditional yield in the
instruction stream. In that case serc can emit an always-true guard.

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
that target's denominator. CPython independently recompiles the strict output
and compares executable fields recursively. For every strict refusal, the
checker requires default mode to emit marked source that CPython can compile.
The checker never executes corpus code.

| CPython   | Compilable modules | Strictly verified | Coverage | Verified code objects / total | Modules with dead suites / live guards |
| --------- | -----------------: | ----------------: | -------: | ----------------------------: | -------------------------------------: |
| 3.12.14   |              2,190 |             1,819 |    83.1% |                7,754 / 11,063 |                                 42 / 1 |
| 3.13.15   |              2,209 |             1,821 |    82.4% |                7,924 / 11,562 |                                 42 / 1 |
| 3.14.5    |              2,246 |             1,834 |    81.7% |                9,769 / 14,082 |                                 49 / 5 |
| 3.15.0rc2 |              2,266 |             1,852 |    81.7% |                9,781 / 14,114 |                                 52 / 8 |

The code-object count includes each module and its nested code. Only objects
from strictly verified modules count here. On 3.14, 190 of the 412 refusals
produced a candidate that failed verification. The most frequent differences
were instructions, exception tables, and constants. Remaining recovery gaps
include nested exception cleanup, overlapping branches and loops, and patterns
that combine different kinds of captures. Each module reports its first refusal.

Run the check with each pinned CPython. It prints JSON counts, first refusal
reasons and the filenames of any verification failures. To compare two builds,
pass the older executable to `--compare-with`. When the new build refuses a
file, the checker recompiles the older build's output with the same CPython. A
previously verified module in the `regressions` map fails the run even if total
coverage increased.

```console
/path/to/python3.14 -I -B scripts/check_decompile_corpus.py target/debug/serc
```

## Best-effort comparison with pycdc

The comparison compiles the top-level `.py` files from each pinned CPython
installation, runs both decompilers on the same caches, and recompiles their
output with that CPython. The sample below used macOS arm64 installations and
pycdc commit `b4289760970dbc399684f1e155ec6d1ea1cc787e`. Standard-library
packages in subdirectories are outside this sample.

| CPython   | Modules | Compilable output (serc / pycdc) | Matching modules (serc / pycdc) | Matching code objects (serc / pycdc / total) |
| --------- | ------: | -------------------------------: | ------------------------------: | -------------------------------------------: |
| 3.12.14   |     164 |                         164 / 51 |                          19 / 3 |                           5,456 / 97 / 8,040 |
| 3.13.15   |     153 |                        153 / 153 |                          21 / 1 |                            4,849 / 1 / 7,720 |
| 3.14.5    |     154 |                          154 / 0 |                          22 / 0 |                            5,110 / 0 / 8,064 |
| 3.15.0rc2 |     152 |                          152 / 0 |                          19 / 0 |                            5,214 / 0 / 8,259 |

A compilable file can contain placeholders. The last column pairs code objects
by qualified name and occurrence and compares executable fields, including
nested code. This counts intact functions in a partially recovered module. These
independent CPython comparisons omit source locations and marshal reference
identity. The serc verifier additionally checks reference sharing. This pycdc
revision rejected the 3.14 and 3.15 cache magic values.

Across the four runs, 112 of the 623 outputs also include detached definitions.
Their 822 commented entries are available for inspection and contribute no
executable code objects to the comparison.

```console
/path/to/python3.14 -I -B scripts/compare_decompile_quality.py target/debug/serc /path/to/pycdc
```

Pass `--corpus stdlib-packages` to compare the installed packages and their
submodules. The script selects package directories from the interpreter's
`sys.stdlib_module_names` and compiles their Python files without importing
them. This excludes third-party distributions in `site-packages`. The JSON
report breaks out executable matches and detached definitions by package. Pass
`--corpus ruff` to compare the two decompilers on the pinned fixtures.
