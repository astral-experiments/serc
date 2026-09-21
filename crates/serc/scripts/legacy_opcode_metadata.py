"""Read pinned CPython 3.12/3.13 opcode metadata without executing source files."""

from __future__ import annotations

import ast
import re
from typing import TypedDict


class OpcodeMetadata(TypedDict):
    opcodes: list[tuple[str, int]]
    caches: dict[str, int]
    flags: dict[str, set[str]]
    popped: dict[str, str]
    pushed: dict[str, str]
    predicates: dict[str, set[str]]


def _numeric_defines(source: str) -> dict[str, int]:
    return {
        name: int(value, 0)
        for name, value in re.findall(
            r"^#define\s+([A-Za-z_][A-Za-z0-9_]*)\s+(0x[0-9a-fA-F]+|\d+)\s*$",
            source,
            re.MULTILINE,
        )
    }


def _assignments(source: str) -> dict[str, ast.expr]:
    return {
        target.id: node.value
        for node in ast.parse(source).body
        if isinstance(node, ast.Assign)
        for target in node.targets
        if isinstance(target, ast.Name)
    }


def _select_opcodes(
    opmap: dict[str, int], header: str, instrumented_min: int
) -> list[tuple[str, int]]:
    definitions = _numeric_defines(header)
    selected = sorted(
        ((name, code) for name, code in opmap.items() if code < instrumented_min),
        key=lambda item: item[1],
    )
    if not {"CACHE", "RESUME", "FOR_ITER", "RETURN_CONST"} <= dict(selected).keys():
        raise ValueError("legacy opcode table is incomplete")
    if len({code for _, code in selected}) != len(selected):
        raise ValueError("duplicate physical opcode ID in legacy table")
    for name, code in opmap.items():
        if definitions.get(name) != code:
            raise ValueError(f"opcode {name} disagrees with its generated header")
    return selected


def _predicate(source: str, name: str) -> set[str]:
    lines = source.splitlines()
    for index, line in enumerate(lines):
        if not line.startswith(f"#define {name}(opcode)"):
            continue
        body = [line]
        while body[-1].rstrip().endswith("\\"):
            index += 1
            body.append(lines[index])
        names = set(re.findall(r"\(opcode\)\s*==\s*([A-Z][A-Z0-9_]*)", "\n".join(body)))
        break
    else:
        raise ValueError(f"missing legacy opcode predicate {name}")
    if not names:
        raise ValueError(f"empty legacy opcode predicate {name}")
    return names


def _finish_metadata(
    sources: dict[str, str],
    opcodes: list[tuple[str, int]],
    flags: dict[str, set[str]],
    metadata_path: str,
    caches_path: str,
) -> OpcodeMetadata:
    names = {name for name, _ in opcodes}
    metadata = sources[metadata_path]
    valid = set(re.findall(r"\[([A-Z][A-Z0-9_]*)\]\s*=\s*\{\s*true,", metadata))
    if missing := names - valid:
        raise ValueError(f"missing legacy opcode validity metadata: {sorted(missing)}")
    caches_match = re.search(
        r"const uint8_t _PyOpcode_Caches\[256\] = \{([\s\S]*?)\};",
        sources[caches_path],
    )
    if caches_match is None:
        raise ValueError("missing legacy inline-cache table")
    caches = {
        name: int(value)
        for name, value in re.findall(
            r"\[([A-Z][A-Z0-9_]*)\]\s*=\s*(\d+),", caches_match.group(1)
        )
    }
    if missing := set(caches) - valid:
        raise ValueError(f"cache entries reference invalid opcodes: {sorted(missing)}")
    effects = {}
    for direction in ("popped", "pushed"):
        match = re.search(
            rf"int\s+_PyOpcode_num_{direction}\(int opcode, int oparg(?:, bool jump)?\)\s*\{{([\s\S]*?)\n\}}",
            metadata,
        )
        if match is None:
            raise ValueError(f"missing legacy {direction} stack function")
        values = {
            name: " ".join(expression.split())
            for name, expression in re.findall(
                r"case\s+([A-Z][A-Z0-9_]*):\s*return\s+([^;]+);", match.group(1)
            )
            if name in names
        }
        if missing := names - values.keys():
            raise ValueError(
                f"missing legacy {direction} stack effects: {sorted(missing)}"
            )
        constants = _numeric_defines(sources["Include/ceval.h"])
        for name, expression in values.items():
            for constant in ("FVS_MASK", "FVS_HAVE_SPEC"):
                if re.search(rf"\b{constant}\b", expression):
                    expression = re.sub(
                        rf"\b{constant}\b", str(constants[constant]), expression
                    )
            # Validate every expression when loading metadata, before rendering Rust.
            rust_stack_expression(expression)
            values[name] = expression
        effects[direction] = values
    utils = sources["Include/internal/pycore_opcode_utils.h"]
    predicates = {
        "is_scope_exit": _predicate(utils, "IS_SCOPE_EXIT_OPCODE") & names,
        "is_unconditional_jump": _predicate(utils, "IS_UNCONDITIONAL_JUMP_OPCODE")
        & names,
        # FOR_ITER and SEND also have jump targets, but are not boolean branches.
        "is_conditional_jump": {
            name for name in names if name.startswith("POP_JUMP_IF_")
        },
    }
    if any(not values for values in predicates.values()):
        raise ValueError("empty legacy opcode predicate")
    return {
        "opcodes": opcodes,
        "caches": {name: width for name, width in caches.items() if name in names},
        "flags": flags,
        "popped": effects["popped"],
        "pushed": effects["pushed"],
        "predicates": predicates,
    }


def parse_312_metadata(sources: dict[str, str]) -> OpcodeMetadata:
    """Join 3.12's Python property declarations and separately generated C tables."""
    source = sources["Lib/opcode.py"]
    assignments = _assignments(source)
    instrumented_min = ast.literal_eval(assignments["MIN_INSTRUMENTED_OPCODE"])
    have_argument = ast.literal_eval(assignments["HAVE_ARGUMENT"])
    opmap = {}
    properties: dict[str, set[int]] = {
        name: set()
        for name in ("hasconst", "hasname", "hasjrel", "hasjabs", "haslocal", "hasfree")
    }
    defining_calls = {
        "def_op": None,
        "name_op": "hasname",
        "jrel_op": "hasjrel",
        "jabs_op": "hasjabs",
    }
    saw_argument_rule = False
    expected_rule = ast.dump(
        ast.parse(
            "[op for op in opmap.values() if op >= HAVE_ARGUMENT]", mode="eval"
        ).body
    )
    for node in ast.parse(source).body:
        if not isinstance(node, ast.Expr) or not isinstance(node.value, ast.Call):
            continue
        call = node.value
        if isinstance(call.func, ast.Name) and call.func.id in defining_calls:
            name, code = (ast.literal_eval(arg) for arg in call.args)
            if not isinstance(name, str) or type(code) is not int or name in opmap:
                raise ValueError("invalid legacy opcode declaration")
            opmap[name] = code
            if property_name := defining_calls[call.func.id]:
                properties[property_name].add(code)
        elif isinstance(call.func, ast.Attribute) and isinstance(
            call.func.value, ast.Name
        ):
            list_name = call.func.value.id
            if list_name == "hasarg":
                if (
                    call.func.attr != "extend"
                    or len(call.args) != 1
                    or ast.dump(call.args[0]) != expected_rule
                ):
                    raise ValueError("unsupported legacy argument property rule")
                saw_argument_rule = True
            elif list_name in properties:
                if call.func.attr != "append" or len(call.args) != 1:
                    raise ValueError(
                        f"unsupported legacy property declaration: {list_name}"
                    )
                code = ast.literal_eval(call.args[0])
                if type(code) is not int:
                    raise ValueError(f"noninteger legacy property: {list_name}")
                properties[list_name].add(code)
    if not saw_argument_rule:
        raise ValueError("missing legacy argument property rule")
    opcodes = _select_opcodes(opmap, sources["Include/opcode.h"], instrumented_min)
    properties["hasarg"] = {code for _, code in opcodes if code >= have_argument}
    property_flags = {
        "hasarg": "HAS_ARG_FLAG",
        "hasconst": "HAS_CONST_FLAG",
        "hasname": "HAS_NAME_FLAG",
        "hasjrel": "HAS_JUMP_FLAG",
        "hasjabs": "HAS_JUMP_FLAG",
        "haslocal": "HAS_LOCAL_FLAG",
        "hasfree": "HAS_FREE_FLAG",
    }
    flags = {
        name: {
            flag for prop, flag in property_flags.items() if code in properties[prop]
        }
        for name, code in opcodes
    }
    return _finish_metadata(
        sources,
        opcodes,
        flags,
        "Python/opcode_metadata.h",
        "Include/internal/pycore_opcode.h",
    )


def parse_313_metadata(sources: dict[str, str]) -> OpcodeMetadata:
    """Select ordinary 3.13 opcodes from opmap, including RESUME above the ID gap."""
    assignments = _assignments(sources["Lib/_opcode_metadata.py"])
    opmap_node = assignments["opmap"]
    if not isinstance(opmap_node, ast.Dict):
        raise ValueError("legacy opmap must be a literal dictionary")
    keys = [ast.literal_eval(key) for key in opmap_node.keys]
    if len(set(keys)) != len(keys):
        raise ValueError("duplicate legacy opcode name")
    opmap = ast.literal_eval(opmap_node)
    if not isinstance(opmap, dict) or any(
        not isinstance(name, str) or type(code) is not int
        for name, code in opmap.items()
    ):
        raise ValueError("invalid legacy opmap")
    instrumented_min = ast.literal_eval(assignments["MIN_INSTRUMENTED_OPCODE"])
    opcodes = _select_opcodes(opmap, sources["Include/opcode_ids.h"], instrumented_min)
    metadata_path = "Include/internal/pycore_opcode_metadata.h"
    flags = {
        name: set(re.findall(r"HAS_[A-Z0-9_]+_FLAG", expression))
        for name, expression in re.findall(
            r"\[([A-Z][A-Z0-9_]*)\]\s*=\s*\{\s*true,\s*[^,]+,\s*([^}]+)\}",
            sources[metadata_path],
        )
        if name in dict(opcodes)
    }
    if missing := dict(opcodes).keys() - flags.keys():
        raise ValueError(f"missing legacy opcode flags: {sorted(missing)}")
    return _finish_metadata(sources, opcodes, flags, metadata_path, metadata_path)


def rust_stack_expression(expression: str) -> str:
    """Translate the integer/boolean expressions in legacy generated stack tables."""
    # Older generators spell conditional array sizes as C's boolean ternary.
    expression = re.sub(r"\(\(([^?;]+?)\)\s*\?\s*1\s*:\s*0\)", r"int(\1)", expression)
    try:
        node = ast.parse(expression, mode="eval").body
    except SyntaxError as error:
        raise ValueError(
            f"unsupported legacy stack expression: {expression}"
        ) from error
    operations = (ast.Add, ast.Sub, ast.Mult, ast.BitAnd, ast.RShift, ast.LShift)

    def translate(value: ast.expr) -> ast.expr:
        if (
            isinstance(value, ast.Constant)
            and type(value.value) is int
            and value.value >= 0
        ):
            return value
        if isinstance(value, ast.Name) and value.id == "oparg":
            return ast.Name(id="argument", ctx=ast.Load())
        if isinstance(value, ast.BinOp) and isinstance(value.op, operations):
            left, right = translate(value.left), translate(value.right)
            if (
                isinstance(value.op, ast.Add)
                and isinstance(left, ast.Constant)
                and isinstance(right, ast.Constant)
            ):
                return ast.Constant(value=left.value + right.value)
            return ast.BinOp(left=left, op=value.op, right=right)
        if (
            isinstance(value, ast.Compare)
            and len(value.ops) == 1
            and isinstance(value.ops[0], ast.Eq)
        ):
            return ast.Compare(
                left=translate(value.left),
                ops=value.ops,
                comparators=[translate(value.comparators[0])],
            )
        if (
            isinstance(value, ast.Call)
            and isinstance(value.func, ast.Name)
            and value.func.id == "int"
            and len(value.args) == 1
            and not value.keywords
        ):
            condition = translate(value.args[0])
            if not isinstance(condition, ast.Compare):
                condition = ast.Compare(
                    left=condition,
                    ops=[ast.NotEq()],
                    comparators=[ast.Constant(value=0)],
                )
            return ast.Call(
                func=ast.Name(id="int", ctx=ast.Load()), args=[condition], keywords=[]
            )
        raise ValueError(f"unsupported legacy stack expression: {expression}")

    return ast.unparse(translate(node)).replace("int(", "i64::from(")
