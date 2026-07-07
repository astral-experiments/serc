# Serc architecture

Serc compiles Python source to CPython code objects without embedding or starting a
Python runtime. Ruff supplies the parser and AST; the rest of the compiler, assembler, marshal
writer, and `.pyc` wrapper are implemented in Rust.

This document describes the boundaries that must remain stable for byte-for-byte compatibility.
It is intentionally specific to the current backend rather than a design for a generic Python
virtual machine.

## Compatibility contract

The only supported target is:

- implementation: `cpython`;
- release: `v3.14.5`;
- source commit: `5607950ef232dad16d75c0cf53101d9649d89115`;
- magic number: `2b0e0d0a`.

`CPYTHON_TARGET` in `src/lib.rs` exposes this identity from the generated target module. The target
includes more than the instruction set: code-object fields, flags, inline caches, line and
exception tables, marshal object identity, and object ordering are all part of the compatibility
surface.

The oracle comparison is:

```python
marshal.dumps(compile(source, filename, "exec", dont_inherit=True, optimize=0))
```

Matching executable behavior is therefore insufficient. The emitted marshal bytes must match the
oracle, including metadata and reference layout.

## Pipeline and ownership

The main path is:

```text
source
  -> normalization and Ruff parsing
  -> scope planning and AST lowering
  -> symbolic instruction emission
  -> CFG optimization and physical stack-size analysis
  -> final bytecode and table layout
  -> CodeObject
  -> marshal object graph and writer
  -> optional timestamp-based .pyc header
```

Each stage has one primary owner:

| Concern                                                                      | Owner              |
| ---------------------------------------------------------------------------- | ------------------ |
| Public library API, input normalization, parser setup, `.pyc` header         | `src/lib.rs`       |
| Filesystem CLI, source discovery, and timestamp-cache writes                 | `../serc_cli/`  |
| Scope analysis, AST lowering, constants/names/locals, logical emission-depth invariants | `src/compiler.rs`, `src/compiler/` |
| Finished constant and code-object model                                      | `src/code_object.rs` |
| Labels, instruction policies/provenance, CFG rewrites, physical stack-size analysis, final layout | `src/assembler/` |
| CPython IDs, widths, flags, stack metadata, version and provenance           | `src/target/`      |
| Marshal equality/identity keys, reference graph, binary encoding             | `src/marshal/`     |

Code should stay on the side of this boundary that owns the relevant CPython behavior. In
particular, numeric target data does not belong in the compiler or assembler, and control-flow
rewrites do not belong in the AST lowering code.

### Parsing

`compile` in `src/lib.rs` normalizes CRLF and bare CR newlines, removes one leading UTF-8 BOM, and
parses a module with Ruff's `PY314` grammar. The shared `CompilationContext` retains Ruff's tokens
alongside the normalized source for source-boundary and reconstruction queries. Ruff returns
syntax and source ranges; it does not provide CPython's symbol table, compiler, runtime, or marshal
implementation.

### Scope planning and lowering

`src/compiler.rs` owns the shared lowering state. The behavioral implementation is split into real
Rust submodules under `src/compiler/`:

- `scope.rs` owns scope plans, name interning, and the binding/reference collectors.
- `orchestration.rs` owns compiler construction, nested compilation, and code-object finishing.
- `statement.rs`, `definition.rs`, and `expression.rs` own their corresponding AST lowering,
    including control flow, annotations, comprehensions, and patterns.
- `symbol.rs`, `constant.rs`, and `source.rs` own name/output emission, constant folding, and
    source-range or annotation reconstruction respectively.
- `tests.rs` contains the compiler unit tests.

In particular, `src/compiler/scope.rs` performs the CPython-specific analysis that lowering needs:

- `LocalCollector` and `ReferenceCollector` collect bindings, explicit globals/nonlocals, and
    references.
- `ModulePlan` records future flags, globals, imports, annotation facts, ordered comprehension
    targets, and comprehension cells while the module name/scope walk is already in progress.
- `FunctionPlan` recursively resolves function locals, cells, free variables, annotation free
    variables, and nested-function requirements before a function body is lowered.
- Module, class, lambda, comprehension, generator, and annotation-scope helpers handle the
    additional name-resolution rules for those scopes.

Module analysis produces the complete `ScopeArena` and `ModulePlan` before the root `Compiler` is
constructed. `CompilationContext` owns both for the lifetime of root and child compilation, so no
lowering path can observe an uninitialized scope arena. The module lowering consumes the plan's
interned `NameId` facts instead of repeating whole-module visitors; the separate local collector is
retained only where CPython source order is observable.

`Compiler` then lowers the Ruff AST into symbolic `Instruction` values. At this point jumps still
refer to `Label`s rather than byte offsets. The compiler also owns constant and name tables, local
layout, exception-region declarations, source locations, and a logical operand-stack depth used to
check emission. That logical depth is not the code object's authoritative physical stack size.
Nested functions, classes, comprehensions, and annotation thunks are recursively compiled into
`Constant::Code` values.

Before assembly, `Compiler::finish_inner`:

1. materializes deferred constants that must precede the implicit return;
1. emits an implicit return or generator exception handler when required;
1. verifies that the logical stack depth is zero;
1. resolves the remaining deferred constant and name indices;
1. performs the compiler-owned constant-pop cleanup and removes unused constants;
1. computes CPython local-kind bytes; and
1. passes the symbolic stream to `Assembler::finish_code`.

Compiler depth tracking ensures that lowering emits balanced instruction sequences and supplies
depth provenance where exception setup edges require it. The assembler independently derives the
physical maximum after optimization.

`src/code_object.rs` owns the finished `Constant` and `CodeObject` model. It is a neutral leaf
shared by the compiler and marshal layers, so binary encoding does not depend on compiler
implementation details. The resulting `CodeObject` is the complete handwritten representation
consumed by the marshal layer.

### Assembler

The assembler pipeline has three ownership phases:

1. `Assembler` is the compiler-facing emission builder. It owns the symbolic instruction stream,
   labels, exception declarations, and compiler-time patching and constant-index operations.
2. `OptimizationPipeline` owns the `BlockGraph` and is the only phase on which CFG and dataflow
   passes are available.
3. `LayoutAssembler` owns the flattened final order, resolves physical instruction widths, and
   encodes bytecode, line tables, and exception tables.

`Assembler::finish_code` consumes the builder into `OptimizationPipeline`, reusing a graph already
prepared for constant reachability or constructing one from the linear stream. The `BlockGraph`
uses an append-only block arena, a separate emission-order vector, and a shared label-to-block
index. `BasicBlockId` therefore remains stable when passes reorder blocks or append duplicated and
synthetic blocks. Pass-local `ItemPosition` cursors may shift when a pass inserts or removes an
item and must not escape that pass. The phase types prevent CFG passes from running while the
compiler can still emit or patch instructions, and prevent block-aware passes after flattening.

The graph remains the owning representation through all control-flow and dataflow passes. Passes
whose CPython block boundary rules are narrower than the structural graph use ordered position
views without moving graph-owned items. Exit duplication allocates stable IDs for copied blocks,
and cold-block collection changes only the emission order. The graph is flattened only after the
last block-aware pass and physical stack-size analysis, immediately before jump-width convergence
and final layout.

An `Instruction` carries its opcode, typed `Operand`, source location, and optional compiler-depth
provenance. Pass-sensitive semantics are represented by dedicated policy types and narrowly named
provenance rather than a generic flag bag:

- `LoadPolicy`, `NoLocationPolicy`, and `FusionPolicy` describe ownership, no-location handling,
  and neighboring-instruction fusion constraints.
- `JumpPolicy` and `RetainedJumpNop` describe threading, exit duplication, redundant removal, and
  line-event preservation.
- `ExceptionPolicy` describes exception-range exclusion and normalized exception ownership.
- `BorrowFallthrough` and the converted-`POP_BLOCK` marker retain the remaining pass-specific
  provenance.

Constructors for emitted, synthetic, and fused instructions initialize or combine each policy
according to its own semantics. A pass that replaces an instruction must therefore make an
explicit policy decision instead of accidentally copying unrelated generic bits.

`Assembler::finish_code` runs the following order. This is a compatibility contract, not a menu of
independent optimizations.

| Order | Pass                                                                       |
| ----: | -------------------------------------------------------------------------- |
|     1 | Remove initially unreachable instructions                                  |
|     2 | Optimize boolean conversions                                               |
|     3 | Thread forward jumps                                                       |
|     4 | Remove instructions made unreachable by threading                          |
|     5 | Remove redundant forward jumps, early pass                                 |
|     6 | Replace overwritten adjacent local stores                                  |
|     7 | Minimize runs of stack swaps                                               |
|     8 | Apply swaps statically to stores and pops                                  |
|     9 | Duplicate eligible exit blocks                                             |
|    10 | Convert potentially uninitialized local loads to checked loads             |
|    11 | Fuse adjacent local-load/store superinstructions                           |
|    12 | Move cold blocks to the end                                                |
|    13 | Remove repeated checked loads within basic blocks                          |
|    14 | Propagate source locations within blocks                                   |
|    15 | Remove redundant swaps before pops                                         |
|    16 | Remove redundant NOPs                                                      |
|    17 | Remove redundant forward jumps, late pass                                  |
|    18 | Convert safe owned local loads to borrowed loads                           |
|    19 | Resolve jump widths, then emit bytecode, line tables, and exception tables |

Several dependencies are explicit in the implementation:

- redundant-store and static-swap optimization must happen before superinstruction fusion;
- borrowed-load conversion runs after the control-flow graph has its final shape;
- instruction policies and provenance must be deliberately preserved, combined according to
    their type-specific rules, or cleared when a pass replaces an instruction; and
- source locations and exception ownership are observable output, even when an opcode becomes a
    `NOP` or is removed.

After the final CFG optimization, `OptimizationPipeline::calculate_stack_size` traverses the final
reachable graph. Target stack effects define each instruction's physical effect; ordinary jump and
fallthrough edges define normal reachability. Exception regions add handler edges at their restored
depths, while retained `ExceptionHandlerEntry` provenance preserves the original `SETUP_*` edge
when optimization has collapsed the corresponding emitted region. The traversal rejects stack
underflow and inconsistent block-entry depths.

This result is the authoritative `CodeObject::stack_size`. The compiler's logical depth remains an
emission invariant and limited source of setup-edge provenance; it is not a pre-optimization
maximum used for the code object. When `OptimizationPipeline` is consumed, layout receives the
stack analysis only as its computed scalar alongside the flattened items and table inputs—no
compiler depth state or CFG depth map crosses into `LayoutAssembler`.

Final layout repeatedly computes label positions and jump arguments over the flattened final order
while accounting for inline caches and `EXTENDED_ARG`. It stops when every instruction's width is
stable, with an eight-pass convergence limit. Only then can line and exception tables be encoded.

In tests and debug builds, stage validation checks:

- every label is bound at most once;
- every jump target and exception-region label is bound;
- forward and backward operands point in the declared direction;
- both reachability-removal stages leave no unreachable instruction; and
- exception-region starts precede their ends at final layout.

These checks diagnose structural corruption; the byte-for-byte oracle remains the semantic check.

### Marshal encoding and `.pyc` output

Marshal encoding is deliberately split into three responsibilities:

- `marshal/key.rs` defines when values compare equal and when CPython preserves object identity. It
    also defines deterministic sort keys for unordered constants such as frozen sets.
- `marshal/graph.rs` walks the complete code-object graph first. It counts repeated objects and
    records strings that CPython will intern.
- `marshal/writer.rs` uses that graph to assign reference indices and encode code objects,
    constants, strings, tuples, local-kind bytes, line tables, and exception tables in CPython's
    order.

The graph pass must precede writing because whether an object's first occurrence receives
`FLAG_REF` depends on later occurrences. Replacing the two-pass design with opportunistic
single-pass serialization changes bytes even when decoded values are equal.

`CompiledModule::to_timestamp_pyc` prefixes the marshalled code object with the generated magic
number, timestamp-invalidation flags, source modification time, and source size. The runtime is
needed only to execute this output or act as the comparison oracle.

## Generated target metadata

`src/target/mod.rs` is handwritten. It defines `Opcode` and keeps its constructor private so the
generated module is the sole numeric authority. Each opcode bundles its numeric ID with its inline
cache width.

`src/target/cpython_3_14_5.rs` is generated from the pinned CPython checkout. It contains:

- target tag, commit, version, implementation, and magic number;
- physical compiled-code opcodes and inline-cache widths;
- opcode metadata flags and control-flow predicates;
- typed operand values for intrinsics, function attributes, common constants, resume points,
  binary and comparison operations, conversions, and special-method loads;
- code flags and local-kind bits; and
- separate dynamic pop and push counts used by assembler dataflow.

`src/marshal/v5.rs` is generated separately from CPython's marshal sources. It contains only the
version and wire tags for marshal format 5; the generic graph, reference, ordering, and encoding
logic remains handwritten in `src/marshal`.

The generator reads these CPython files:

- `Include/opcode_ids.h`;
- `Include/opcode.h`;
- `Include/object.h`;
- `Include/ceval.h`;
- `Include/internal/pycore_opcode_metadata.h`;
- `Include/internal/pycore_opcode_utils.h`;
- `Include/internal/pycore_intrinsics.h`;
- `Include/internal/pycore_ceval.h`;
- `Include/cpython/code.h`;
- `Include/internal/pycore_code.h`;
- `Include/internal/pycore_magic_number.h`;
- `Include/patchlevel.h`;
- `Python/codegen.c`;
- `Python/flowgraph.c`;
- `Include/marshal.h`; and
- `Python/marshal.c`.

Run it from the repository root:

```console
python3 crates/serc/scripts/generate_cpython_target.py \
  --cpython /path/to/cpython
python3 crates/serc/scripts/generate_cpython_target.py \
  --cpython /path/to/cpython \
  --check
```

The checkout must be at the exact pinned commit. Generation verifies the commit, the release tag
when available, the expected version, opcode, operand, predicate, stack, and marshal-metadata
completeness, and cache references. Each output records a SHA-256 digest for its input sources and
is formatted before comparison. `--check` also rejects direct `Opcode::new` calls and duplicated
numeric opcode constants in the compiler and assembler.

Do not hand-edit the generated files. Internal compiler state that cannot appear in output should
be represented by typed Rust state or instruction flags, not by inventing a private bytecode
opcode.

## Correctness gate

Focused tests can use a CPython 3.14.5 executable through
`SERC_TEST_PYTHON`. The full gate compares the Ruff and ty corpora to CPython's
marshal output. From the repository root, with `RUFF` pointing to a Ruff checkout:

```console
SERC_TEST_PYTHON=/path/to/python3.14 \
  cargo test --locked --workspace --all-targets

cargo run --locked -p serc --example compare_cpython_corpus -- \
  --python /path/to/python3.14 \
  --expected-files 2451 \
  --require-all \
  "$RUFF/crates/ty_python_semantic/resources/corpus" \
  "$RUFF/crates/ruff_python_parser/resources/valid" \
  "$RUFF/crates/ruff_python_parser/resources/inline/ok" \
  "$RUFF/crates/ruff_linter/resources/test/fixtures" \
  "$RUFF/crates/ruff_python_formatter/resources/test/fixtures"
```

The comparator verifies the oracle's implementation, three-part version, and magic number before
running. It also pins the discovered file count. CPython-rejected fixtures are reported separately;
every CPython-accepted file must match exactly under `--require-all`.

The frozen baseline is 2,230 exact matches and 221 CPython-rejected files, with zero byte
mismatches, unsupported files, parse mismatches, compiler panics, non-UTF-8 files, or oracle
failures.

## Safe changes

For a behavior-preserving refactor:

1. Keep each change inside the ownership boundary above.
1. Preserve assembler pass order unless changing it is the purpose of the change.
1. Preserve instruction provenance, source locations, exception-region ownership, table ordering,
    and marshal identity rules; they are observable even when execution is unchanged.
1. Add a focused regression for any newly understood edge case.
1. Run formatting, the generated-metadata check, all-target tests against the exact oracle, and the
    full corpus gate.

For a new CPython target:

1. Select and record an exact release tag and commit.
1. Update the generator's pin, expected version, and output module; wire the new module through
    `src/target/mod.rs` and regenerate it.
1. Audit the matching CPython compiler, flow-graph optimizer, assembler, code-object, and marshal
    sources. Generated metadata covers data tables, not handwritten semantics.
1. Update Ruff's parser target, public target identity, documentation, tests, and oracle selection
    as required by the Python version.
1. Run the full gate and investigate every changed byte. Change the frozen corpus baseline only
    when the corpus or target intentionally changes.

Treat a green focused test as necessary but not sufficient. Any change to lowering, pass ordering,
layout, constants, names, locals, or marshal identity can be correct on small examples while still
changing bytes elsewhere in the corpus.
