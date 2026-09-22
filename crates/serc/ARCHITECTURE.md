# Serc architecture

Serc compiles Python source to CPython code objects without starting a Python runtime. Ruff
supplies the parser and AST; lowering, assembly, marshal encoding, and `.pyc` output are Rust.

```text
source -> decoding/normalization -> Ruff AST -> scope planning/lowering
       -> symbolic instructions -> assembly -> CodeObject -> marshal/.pyc
```

## Ownership

| Concern | Owner |
| --- | --- |
| Public API, target selection, source decoding, `.pyc` header | [lib.rs](src/lib.rs), [config.rs](src/config.rs), [source.rs](src/source.rs) |
| Filesystem traversal and timestamp-cache writes | [serc_cli](../serc_cli/) |
| Scope planning, AST lowering, constants/names/locals, logical stack depth | [compiler](src/compiler/) |
| Finished code-object fields and marshal provenance | [code_object.rs](src/code_object.rs) |
| Control flow, instruction layout, line and exception tables | [assembler](src/assembler/) |
| CPython numeric metadata and target identity | [target](src/target/) |
| Object identity, reference planning, binary encoding | [marshal](src/marshal/) |

Keep target numbers in generated metadata, control-flow rewrites in the assembler, and binary
encoding in marshal. The compiler records the source and identity information those stages need.

## Compatibility contract

Compatibility means identical bytes for the selected pinned CPython target, including code-object
fields, table order, source locations, exception ownership, and marshal references. Equivalent
execution or equal decoded values are insufficient. See the [compatibility gates](../../docs/compatibility.md)
for oracle selection, corpus pins, and accepted limitations.

[`CompileOptions`](src/config.rs) selects Python 3.12–3.15; the same selection controls parsing,
lowering, assembly, and serialization. `PythonVersion::target` exposes the exact release identity.
The compiled module retains its target and initial intern state so later serialization uses the
same inputs.

## Parsing and lowering

The [source layer](src/source.rs) decodes and normalizes input before [lib.rs](src/lib.rs) selects
Ruff's grammar for the requested target. The [library API](README.md#library-source-api) documents
encoding and normalization behavior. Ruff provides syntax and ranges; Serc implements CPython's
scope and compilation rules.

[`CompilationContext`](src/compiler/source.rs) retains normalized source, tokens, and a complete
[`ScopeArena`](src/compiler/scope.rs) before root or child lowering begins. Nested code shares this
context. Source-order-dependent collection and interning chronology must survive refactoring,
including effects of code that is later discarded.

The compiler emits symbolic labels and instructions, manages constant/name/local table order,
declares exception regions, and tracks logical stack depth. Nested scopes become `Constant::Code`
values. [Finishing](src/compiler/orchestration.rs) resolves deferred operands and passes the stream
and constant table to assembly. [`CodeObject`](src/code_object.rs) retains both output fields and
marshal provenance; replacing provenance with value equality can change serialized identity.

## Assembly

The [pipeline](src/assembler/passes/mod.rs) selects the target-specific pass sequence, including the
[3.12/3.13 path](src/assembler/passes/legacy.rs) and [linear fast path](src/assembler/passes/linear.rs).
Pass order and traversal order are compatibility requirements. Consult those implementations when
changing a pass; their order is not a set of interchangeable optimizations.

The graph path stores blocks in an append-only [`BlockGraph`](src/assembler/block_graph.rs), with
separate emission order and label lookup. `BasicBlockId` remains stable through reordering and
copying; `ItemPosition` is a temporary cursor invalidated by edits within its block. Passes with
different block-boundary rules use views of the graph rather than taking ownership of its items.

In particular:

- Redundant-store and static-swap rewrites precede superinstruction fusion.
- Borrowed-load analysis runs after control-flow rewrites establish the final shape.
- Instruction replacement must deliberately preserve, combine, or clear provenance. Source
  locations and exception owners remain observable when an instruction becomes a `NOP` or is
  removed.

Final layout resolves label positions and jump widths with the selected target's inline caches
and `EXTENDED_ARG` rules before emitting bytecode, line tables, and exception tables. Structural
checks catch invalid labels, jump directions, reachability, and region boundaries; only the oracle
comparison establishes byte compatibility.

## Marshal and `.pyc`

[`key.rs`](src/marshal/key.rs) separates equality from identity and defines frozen-set wire order.
[`plan.rs`](src/marshal/plan.rs) owns the traversal shared by [graph analysis](src/marshal/graph.rs)
and the [writer](src/marshal/writer.rs). Analysis assigns object IDs, counts occurrences, and
determines interning before the writer assigns reference indices and emits bytes.

Analysis must precede writing: a later occurrence can require `FLAG_REF` on an object's first
occurrence. Both passes prune repeated containers consistently, and the occurrence cursor verifies
that writing consumes every planned ID. An opportunistic single-pass serializer loses that
contract. Preserve initial intern-state semantics and version-specific identity rules as well as
field and reference order.

`CompiledModule::to_timestamp_pyc` writes the selected target's magic number, invalidation flags,
modification time, and source size. Its marshal root-reference policy follows `py_compile`, which
can differ from serializing a retained code object with `CompiledModule::marshal`.

## Target metadata and changes

[Generated target modules](src/target/) supply wire opcodes, cache widths, flags, operands, stack
effects, and release provenance; [generated marshal modules](src/marshal/) supply wire tags. Do not
hand-edit them or duplicate their numeric constants in lowering and assembly. Typed internal state
and provenance carry information that is absent from bytecode.

Generation verifies an exact CPython checkout, but generated tables do not replace auditing its
compiler and serialization semantics. Follow [target updates](README.md#updating-the-cpython-target)
for regeneration and [development checks](../../docs/development.md) and
[compatibility gates](../../docs/compatibility.md) for validation.
