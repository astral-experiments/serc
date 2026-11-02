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
shared successors to combine short-circuit tests. When CPython gives several
edges separate copies of a function's implicit `None` return, recovery can place
a guard at function exit without introducing a shared return block. Loop
recovery distinguishes exhaustion from explicit breaks, including when CPython
copied the return after the loop into a break arm. A branch can also leave a
loop by falling through after its last backward jump. More complex cyclic graphs
and exception layouts can still be refused.

A return in a nested guard can leave the enclosing guard's other path open. In
`asyncio.base_events._run_until_complete_cb`, the test of the future's exception
and the test for cancellation both have an edge to the call that stops the loop.
The decompiler puts that call after the guard. All four targets reproduce the
original callback.

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
`await` and distinguishes `with` from `async with` at each entry and exit. When
evaluating a return expression raises, a synchronous manager can suppress the
exception and execution resumes after the `with`. This recovers the second parse
attempt in 3.14.5's `_pyrepl._module_completer.ImportParser._parse`.

Match recovery handles value and singleton patterns, alternatives, guards,
captures, and selected sequence, mapping, and class patterns. Nested or
overlapping cases can still exceed these recognizers.

In `asyncio.__getattr__` on 3.14 and 3.15, two platform checks in literal cases
can reach the same `raise AttributeError` after the match. A failed platform
check leaves the match even if a later case could match the same value. Both
targets reproduce the function's executable fields.

In `asyncio.Queue._wakeup_next`, a backedge follows the test of the current
waiter but returns to the test of the queue. On 3.12 and 3.13, CPython emits a
second copy of the queue test at the bottom of the loop. On 3.14 and 3.15, the
jump targets the top of the loop directly. Recovery checks that destination so a
completed waiter advances the loop to the next item. In 3.14's
`asyncio.graph._build_graph_for_future`, two arms jump back to the header. The
third falls past both jumps, so recovery emits a `break`. The same layout occurs
in the interactive interpreter's dotted-name parser and backspace handler.

A return inside a `for` loop can remove the iterator from beneath a computed
return value. Recovery checks how many slots the target uses for the iterator.
In `asyncio.Semaphore._wake_up_next`, it preserves the early return after waking
a waiter and the separate return on exhaustion on all four targets.

For a protected `await`, CPython temporarily sends throws from `YIELD_VALUE` to
`CLEANUP_THROW`. Both the resumed path and the cleanup can reach the user's
handler. Following those edges places the cancellation handlers inside the loops
in `asyncio.Queue.get` and `Queue.put`, including the nested `except ValueError`
around removal of the waiter. Both methods recompile to matching executable
fields on all four targets. For `finally`, recovery compares the cleanup on the
normal path with the exceptional copy, including the destinations of jumps
within each copy. This recovers the cleanup in `asyncio.sleep` and
`asyncio.Event.wait` across all four targets.

In ordinary functions, a conditional can skip an entire protected suite. The
decompiler places each recognized handler at its protected entry and follows
normal successors to find the end of the function body. This recovers both
guarded callbacks in 3.14.5's `_pyrepl.Reader.run_hooks`. Its `readline` method
also keeps the polling loop and computed return inside the `try` that restores
the reader.

For a chain of `except` clauses, the false edge of each type check identifies
the next clause. Successful handlers can return, raise, or jump back to a shared
continuation. In `asyncio.selector_events`, this separates transient socket
errors from exceptions the callback must re-raise or pass to the future. The
reconstructed `_sock_recv`, `_sock_recv_into`, and `_sock_accept` methods match
on all four targets.

An exception handler in a `for` loop can return from the function or advance to
the next item. In IDLE's `ObjectTreeItem.GetSubList`, an `AttributeError` from
reading one attribute skips that item. Recovery keeps the loop's `continue` in
the handler so the remaining attributes are still inspected.

CPython can build long dictionary displays one pair at a time. Recognizing those
insertions lets recovery continue past the configuration dictionary in 3.14.5's
`turtle.py`, where 212 of 250 code objects now match. In `codecs.py`, the module
begins with a handler that binds an exception and always raises. The exception
table separates the handler body from CPython's cleanup of the bound name.
Following its normal continuation recovers 93 of 100 code objects.

In 3.14.5's `importlib._bootstrap_external`, the module starts with
`assert all(len(sep) == 1 for sep in path_separators)`. CPython emits two paths,
an inlined loop for the builtin and an ordinary call for a replacement bound to
`all`. Recovering that call and the formatted set comprehension that follows it
places the later definitions back in the module. CPython reproduces 99 of its
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
object. A logical instruction starts at its first `EXTENDED_ARG` prefix, if
present, and includes its inline caches. The listing includes the whole block
when recovery stops mid-block, and can include earlier blocks for context when
an incomplete suite is discarded. A lost class or anonymous function referenced
by the skipped instructions can have its own listing. The appendix has a
separate 16 MiB output budget. Reaching it leaves a truncation comment in the
output.

A function can have a complete-looking body that omits a branch or an exception
path. When its normal control flow or exception table differs after
recompilation at all three optimization levels, serc warns on the definition and
lists the original blocks. The body remains executable. The normal-flow check
compares the jump and fallthrough destinations of each block in instruction
order and distinguishes returns from raises. If those edges and the exception
table match, differences in instructions or their operands do not produce a
function warning. Functions with an explicit gap already have a listing.

If the module's own normal flow or exception table differs at every optimization
level, the best-effort header names the difference. The appendix lists the
module's original blocks once, including any blocks already covered by a gap.
The comparison examines the module's import-time instructions separately from
the code in its classes and functions.

Use `serc decompile --strict FILE.pyc` to require whole-module verification. In
strict mode a refusal writes the reason to stderr and leaves stdout empty.
Library callers can use `serc::decompile_pyc_with_mode` to inspect
`verification_error` or select `DecompileMode::Strict`.

Verified output supplies one spelling that produces those executable fields.
Comments, original formatting, location tables, and code removed by CPython
optimization generally cannot be recovered. Traceback line numbers can change.

Since 3.13, a class exposes its first line as `__firstlineno__`. Strict
verification checks this executable constant. Best-effort output also inserts
blank lines before class definitions, including their decorators, to reproduce
it. Serc pairs classes by qualified name and occurrence within the same parent.
It skips ambiguous occurrences and leaves an already aligned class in place. A
single insertion is limited to 1,024 lines, and the padded source must fit the 1
MiB source budget. Blank lines do not count against the space for detached
bodies or original block listings.

In the 3.14.5 standard-library package sample, aligning best-effort output
reproduced `__firstlineno__` for 1,844 of 1,886 original classes, up from 784.
The change added 39,542 blank lines across 193 files.

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
| 3.12.14   |              2,190 |             1,880 |    85.8% |                8,206 / 11,063 |                                 48 / 1 |
| 3.13.15   |              2,209 |             1,884 |    85.3% |                8,378 / 11,562 |                                 48 / 1 |
| 3.14.5    |              2,246 |             1,876 |    83.5% |               10,094 / 14,082 |                                 53 / 5 |
| 3.15.0rc2 |              2,266 |             1,896 |    83.7% |               10,130 / 14,114 |                                 57 / 8 |

The code-object count includes each module and its nested code. Only objects
from strictly verified modules count here. On 3.14, 168 of the 370 refusals
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
| 3.12.14   |     164 |                         164 / 51 |                          24 / 3 |                           6,364 / 97 / 8,040 |
| 3.13.15   |     153 |                        153 / 153 |                          25 / 1 |                            6,015 / 1 / 7,720 |
| 3.14.5    |     154 |                          154 / 0 |                          24 / 0 |                            6,204 / 0 / 8,064 |
| 3.15.0rc2 |     152 |                          152 / 0 |                          21 / 0 |                            6,333 / 0 / 8,259 |

A compilable file can contain placeholders. The last column pairs code objects
by qualified name and occurrence and compares executable fields, including
nested code. This counts intact functions in a partially recovered module. These
independent CPython comparisons omit source locations and marshal reference
identity. The serc verifier additionally checks reference sharing. This pycdc
revision rejected the 3.14 and 3.15 cache magic values.

Across the four runs, 70 of the 623 outputs also include detached definitions.
Their 594 commented entries are available for inspection and contribute no
executable code objects to the comparison.

```console
/path/to/python3.14 -I -B scripts/compare_decompile_quality.py target/debug/serc /path/to/pycdc
```

Pass `--corpus stdlib-packages` to compare the installed packages and their
submodules. The script selects package directories from the interpreter's
`sys.stdlib_module_names` and compiles their Python files without importing
them. This excludes third-party distributions in `site-packages`. The JSON
report breaks out executable matches and detached definitions by package.

| CPython   | Modules | Compilable output (serc / pycdc) | Matching modules (serc / pycdc) | Matching code objects (serc / pycdc / total) |
| --------- | ------: | -------------------------------: | ------------------------------: | -------------------------------------------: |
| 3.12.14   |     519 |                        519 / 257 |                        305 / 25 |                      10,043 / 1,076 / 11,772 |
| 3.13.15   |     477 |                        477 / 404 |                        260 / 14 |                          9,954 / 14 / 11,740 |
| 3.14.5    |     498 |                          498 / 0 |                         263 / 0 |                          10,721 / 0 / 12,586 |
| 3.15.0rc2 |     539 |                          539 / 0 |                         279 / 0 |                          11,430 / 0 / 13,544 |

On 3.14.5, the largest gaps by number of unmatched code objects are in `idlelib`
(255), `asyncio` (241), and `multiprocessing` (163). A mismatch counts once per
code object, so a single lost method can also cause its enclosing class and
module to miss.

An independent CFG check recompiled the 235 best-effort package modules on
3.14.5 at all three optimization levels. It paired uniquely named functions
whose recovered body had no marked gap. Of 2,845 such functions with a
conditional branch and no exception table, 20 had different normal edges at
every level. Of 921 functions with an exception table in the original or a
candidate, 144 had different normal edges and 197 had a different exception
table at every level. A matching graph does not check instruction operands.

On the same target, 21 of the 498 package modules produced a warning about the
module's own code. Four had a changed exception table and 17 also had a changed
normal control-flow graph. In `ctypes._endian`, recovery keeps the
unknown-byte-order exception on its original branch. The module's import-time
graph matches even though code inside some classes still differs.

Add `--compare-with /path/to/older/serc` to check for regressions in individual
functions as well as modules. The report lists previously matching scopes by
filename, qualified name, and occurrence, and the command exits nonzero if any
were lost.

For functions fully placed by both builds, `--compare-with` also checks normal
edges and exception tables independently. Each field matches if native CPython
reproduces it at any of the three optimization levels. Losing a previously
matching field adds the function and field to `flow_regressions` and fails a
standard-library comparison. Ruff reports these changes without failing.
Repeated definitions are paired within their parent when the counts agree. If a
previously placed function is missing, ambiguous, or now contains a marked gap,
it appears in `newly_unplaced`. That list is informational.

The `pycdc_only` map lists every scope that pycdc matched and serc missed. Any
entry fails either standard-library comparison, even if serc matches more scopes
overall. Both standard-library samples passed on all four targets.

The Ruff sample (`--corpus ruff`) uses the same revision and fixture directories
as the strict check. Its `pycdc_only` entries are informational. There were
seven on 3.12 and one on 3.13.

| CPython   | Modules | Compilable output (serc / pycdc) | Matching modules (serc / pycdc) | Matching code objects (serc / pycdc / total) |
| --------- | ------: | -------------------------------: | ------------------------------: | -------------------------------------------: |
| 3.12.14   |   2,190 |                    2,190 / 1,569 |                     1,881 / 754 |                      10,055 / 3,194 / 11,063 |
| 3.13.15   |   2,209 |                    2,209 / 2,040 |                     1,885 / 337 |                        10,413 / 337 / 11,562 |
| 3.14.5    |   2,246 |                        2,246 / 0 |                       1,877 / 0 |                          12,642 / 0 / 14,082 |
| 3.15.0rc2 |   2,266 |                        2,266 / 0 |                       1,897 / 0 |                          12,681 / 0 / 14,114 |
