#!/usr/bin/env python3
"""Derive neutral CPython 3.14 bootstrap strings without executing bootstrap code."""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import re
import sys
from pathlib import Path
from types import CodeType

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_cpython_target as target

MODULES = {
    "Lib/importlib/_bootstrap.py": "<frozen importlib._bootstrap>",
    "Lib/importlib/_bootstrap_external.py": "<frozen importlib._bootstrap_external>",
    "Lib/encodings/__init__.py": "encodings/__init__.py",
    "Lib/encodings/aliases.py": "encodings/aliases.py",
}
PROOF_INPUTS = (
    "Objects/codeobject.c",
    "Python/import.c",
    "Python/pylifecycle.c",
    "Python/codecs.c",
    "Programs/_freeze_module.c",
    "Include/internal/pycore_global_strings.h",
    "Python/Python-ast.c",
)
INPUTS = (*MODULES, *PROOF_INPUTS)
OUTPUT = target.CRATE_ROOT / "scripts/target_metadata/cpython_3_14_5_bootstrap.json"


def code_tree(code: CodeType):
    """Every nested code object is constructed when the containing code is loaded."""
    yield code
    for constant in code.co_consts:
        if isinstance(constant, CodeType):
            yield from code_tree(constant)


def identifier_constants(value):
    """Mirror default-GIL codeobject.c's mortal ASCII constant interning rule."""
    if isinstance(value, str):
        if re.fullmatch(r"[a-zA-Z0-9_]*", value):
            yield value
    elif isinstance(value, (tuple, frozenset, list, dict)):
        for item in value:
            yield from identifier_constants(item)
        if isinstance(value, dict):
            for item in value.values():
                yield from identifier_constants(item)


def linux_statements(body: list[ast.stmt]):
    """Select the source's Linux branches without evaluating arbitrary expressions."""
    for node in body:
        if isinstance(node, ast.If):
            condition = ast.unparse(node.test)
            if condition not in {"_MS_WINDOWS", "sys.platform == 'win32'"}:
                raise ValueError(f"unverified bootstrap condition: {condition}")
            yield from linux_statements(node.orelse)
        else:
            yield node


def verify_decorator(node: ast.expr) -> None:
    """These pinned descriptors/wrappers retain the decorated function object."""
    if isinstance(node, ast.Name) and node.id in {
        "classmethod",
        "staticmethod",
        "property",
        "_requires_builtin",
        "_requires_frozen",
        "_check_name",
    }:
        return
    if isinstance(node, ast.Attribute) and node.attr == "setter":
        return
    raise ValueError(f"unverified retaining decorator: {ast.unparse(node)}")


def derive_module(source: str, filename: str) -> dict:
    tree = ast.parse(source, filename=filename)
    code = compile(source, filename, "exec", dont_inherit=True, optimize=0)
    all_codes = tuple(code_tree(code))
    immortal = {
        value
        for child in all_codes
        for value in (
            *child.co_names,
            *child.co_varnames,
            *child.co_cellvars,
            *child.co_freevars,
        )
    }
    retained = set()
    literals = set()
    owners = []

    def find_code(node):
        first_line = min([node.lineno, *(d.lineno for d in node.decorator_list)])
        matches = [
            child
            for child in all_codes
            if child.co_firstlineno == first_line and child.co_name == node.name
        ]
        if len(matches) != 1:
            raise ValueError(f"ambiguous code owner: {node.name}:{first_line}")
        return matches[0]

    def retain_function(node):
        for decorator in node.decorator_list:
            verify_decorator(decorator)
        root = find_code(node)
        owners.append(f"{root.co_qualname}:{root.co_firstlineno}")
        for child in code_tree(root):
            retained.update((child.co_name, child.co_qualname))
            retained.update(identifier_constants(child.co_consts))
            if child.co_filename.startswith("<frozen "):
                retained.add(child.co_filename)

    def retain_scope(body):
        literal_owners = {}
        function_owners = set()
        for node in linux_statements(body):
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if node.name in function_owners and not any(
                    isinstance(decorator, ast.Attribute)
                    and decorator.attr == "setter"
                    and isinstance(decorator.value, ast.Name)
                    and decorator.value.id == node.name
                    for decorator in node.decorator_list
                ):
                    raise ValueError("bootstrap function owner was replaced")
                function_owners.add(node.name)
                literal_owners.pop(node.name, None)
                retain_function(node)
            elif isinstance(node, ast.ClassDef):
                if node.decorator_list:
                    raise ValueError("unverified retaining class decorator")
                # The class owns __name__/__qualname__, but not its body code.
                child = find_code(node)
                retained.update((child.co_name, child.co_qualname))
                function_owners.add(node.name)
                literal_owners.pop(node.name, None)
                retain_scope(node.body)
            elif isinstance(node, ast.Assign):
                names = {
                    target.id for target in node.targets if isinstance(target, ast.Name)
                }
                if names & function_owners:
                    raise ValueError("bootstrap function owner was replaced")
                try:
                    values = set(identifier_constants(ast.literal_eval(node.value)))
                except (ValueError, TypeError):
                    values = set()
                # A later assignment releases the previous literal owner.
                for name in names:
                    literal_owners[name] = values
            elif isinstance(node, (ast.Delete, ast.For, ast.While, ast.Try, ast.With)):
                raise ValueError("unverified bootstrap owner lifetime")
        for node in ast.walk(ast.Module(body=body, type_ignores=[])):
            if isinstance(node, ast.Global):
                if function_owners.intersection(node.names):
                    raise ValueError("bootstrap function owner may be replaced")
                for name in node.names:
                    literal_owners.pop(name, None)
        for values in literal_owners.values():
            literals.update(values)

    retain_scope(tree.body)
    return {
        "code_objects": len(all_codes),
        "retained_code_owners": sorted(owners),
        "immortal_identifiers": sorted(immortal),
        "retained_code_strings": sorted(retained),
        "retained_literal_strings": sorted(literals),
    }


def generate(checkout: Path) -> dict:
    if sys.implementation.name != "cpython" or tuple(sys.version_info) != (
        3,
        14,
        5,
        "final",
        0,
    ):
        raise ValueError("bootstrap generation requires exact CPython 3.14.5")
    target.select_target_version("3.14")
    target.verify_checkout(checkout)
    sources = {
        relative: target.read_committed_file(checkout, relative) for relative in INPUTS
    }
    modules = {
        relative: derive_module(sources[relative], filename)
        for relative, filename in MODULES.items()
    }
    strings = set()
    for module in modules.values():
        for category in (
            "immortal_identifiers",
            "retained_code_strings",
            "retained_literal_strings",
        ):
            strings.update(module[category])
    # Keep the source-derived supplement independent of the historical startup
    # list. Static/compiler names are already represented in the target metadata.
    static = target.parse_preinterned_strings(
        target.read_committed_file(checkout, "Include/internal/pycore_global_strings.h")
    )
    compiler = target.parse_compiler_interned_strings(
        target.read_committed_file(checkout, "Python/Python-ast.c")
    )
    strings.difference_update(static, compiler)
    strings = sorted(value for value in strings if len(value) > 1)
    return {
        "target_tag": target.TARGET_TAG,
        "target_commit": target.TARGET_COMMIT,
        "platform": "linux",
        "method": "compile-only; no bootstrap module execution or corpus inputs",
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "input_sha256": {
            relative: hashlib.sha256(source.encode()).hexdigest()
            for relative, source in sources.items()
        },
        "modules": modules,
        "supplemental_entries": len(strings),
        "strings": strings,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cpython", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    options = parser.parse_args()
    generated = json.dumps(generate(options.cpython), indent=2) + "\n"
    if options.check:
        if OUTPUT.read_text() != generated:
            raise ValueError(f"stale bootstrap metadata: {OUTPUT}")
    else:
        OUTPUT.write_text(generated)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
