"""Compare recompiled function CFGs and exception tables with the running CPython."""

from __future__ import annotations

import ast
import dis
import inspect
import io
import tokenize
import types
from bisect import bisect_left
from collections import Counter, defaultdict

JUMPS = set(dis.hasjump if hasattr(dis, "hasjump") else dis.hasjrel + dis.hasjabs)
UNCONDITIONAL = {"JUMP_FORWARD", "JUMP_BACKWARD", "JUMP_BACKWARD_NO_INTERRUPT"}
RETURNS = {"RETURN_CONST", "RETURN_VALUE"}
RAISES = {"RERAISE", "RAISE_VARARGS"}
FIELDS = {"normal_flow", "exception_table"}


def normal_flow(code):
    """Number blocks in instruction order and retain their normal exit edges."""
    ops = list(dis.get_instructions(code))
    offsets = {op.offset: i for i, op in enumerate(ops)}
    starts = {0, len(ops)}
    jumps = {}
    for i, op in enumerate(ops):
        # END_ASYNC_FOR's argument describes exception cleanup.
        if op.opcode in JUMPS and op.opname != "END_ASYNC_FOR":
            jumps[i] = offsets[op.argval]
            starts.update((jumps[i], i + 1))
        elif op.opname in RETURNS or op.opname in RAISES:
            starts.add(i + 1)
    starts = sorted(starts)
    labels = {start: block for block, start in enumerate(starts)}
    edges = []
    for block, end in enumerate(starts[1:]):
        i = end - 1
        name = ops[i].opname
        jump = labels[jumps[i]] if i in jumps else None
        terminal = name in RETURNS or name in RAISES or name in UNCONDITIONAL
        edges.append((None if terminal else block + 1, jump, name in RAISES))
    return tuple(edges)


def scopes(code):
    yield code
    for child in code.co_consts:
        if isinstance(child, types.CodeType):
            yield from scopes(child)


def children(code):
    groups = defaultdict(list)
    for child in code.co_consts:
        if isinstance(child, types.CodeType):
            groups[child.co_qualname].append(child)
    return groups


def paired(left, right):
    """Pair siblings in constant-table order only when their counts agree."""
    peers = children(right)
    for name, originals in children(left).items():
        others = peers.get(name, ())
        if len(originals) == len(others):
            for original, candidate in zip(originals, others, strict=True):
                yield original, candidate
                yield from paired(original, candidate)


def complete_definitions(source):
    gaps = sorted(
        token.start[0]
        for token in tokenize.tokenize(io.BytesIO(source).readline)
        if token.type == tokenize.COMMENT
        and token.string.startswith("# serc: unrecovered ")
    )
    complete = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            start = min((d.lineno for d in node.decorator_list), default=node.lineno)
            i = bisect_left(gaps, start)
            if i == len(gaps) or gaps[i] > node.end_lineno:
                complete.add((node.name, start))
    return complete


def function_matches(original, recovered, source, filename):
    complete = complete_definitions(source)
    counts = Counter()
    keys = {}
    for code in scopes(original):
        counts[code.co_qualname] += 1
        keys[id(code)] = (code.co_qualname, counts[code.co_qualname])

    matches = {}
    shapes = {}
    for level in range(3):
        if level:
            recovered = compile(
                source, filename, "exec", dont_inherit=True, optimize=level
            )
        for left, right in paired(original, recovered):
            key = keys[id(left)]
            if (
                not left.co_flags & inspect.CO_OPTIMIZED
                or (right.co_name, right.co_firstlineno) not in complete
            ):
                continue
            # Only functions present at level zero enter the comparison.
            if level and key not in matches:
                continue
            fields = matches.setdefault(key, set())
            if left.co_exceptiontable == right.co_exceptiontable:
                fields.add("exception_table")
            if "normal_flow" not in fields:
                if left.co_code == right.co_code:
                    fields.add("normal_flow")
                else:
                    if key not in shapes:
                        shapes[key] = normal_flow(left)
                    if shapes[key] == normal_flow(right):
                        fields.add("normal_flow")
        if all(fields == FIELDS for fields in matches.values()):
            break
    return matches
