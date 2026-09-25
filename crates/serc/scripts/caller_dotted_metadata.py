"""Lexical code-name candidates before optimizer pruning.

This helper contributes an overapproximation to a source-derived candidate
inventory. Its output proves neither retained interning nor absence of other
names. The reviewed inputs are the selected CPython 3.13/3.15 startup modules;
expanding those inputs still requires reviewing compiler-generated scopes.
"""

from __future__ import annotations

import ast


def lexical_qualnames(tree: ast.AST) -> set[str]:
    """Include named scopes and possible global resets, including dead code.

    CPython's compiler_set_qualname uses the raw child name; private mangling
    only affects its GLOBAL_EXPLICIT lookup. Include both reset and nested
    alternatives without inferring that symbol-table decision.

    Annotation helpers, type-parameter bound/default helpers, and type-alias
    bodies are included by name. The caller filters to its dotted-name grammar:
    lambda, comprehension, and generic-parameter wrapper names contain excluded
    angle-marked segments. This is deliberately not a general compiler model.
    """
    values = set()

    def child_names(name, parents, kind):
        names = {name}
        for parent in parents:
            if parent:
                prefix = parent + (".<locals>" if kind == "function" else "")
                names.add(prefix + "." + name)
        values.update(names)
        return names

    def annotation_helpers(parents, kind):
        for name in ("__annotate__", "__annotate_func__"):
            child_names(name, parents, kind)
            # 3.15 ste_function_name attaches the function's raw name without
            # an additional <locals> segment. Include both forms, and also the
            # class storage key __annotate_func__, as candidate-only extras.
            child_names(name, parents, "class")

    def visit(node, parents, kind):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names = child_names(node.name, parents, kind)
            child_kind = "class" if isinstance(node, ast.ClassDef) else "function"
            annotation_helpers(names, child_kind)
            # Bound/default helper scopes use the parameter's raw name after
            # compiler_set_qualname skips the generic-parameter wrapper.
            for parameter in node.type_params:
                child_names(parameter.name, parents, kind)
            for field, value in ast.iter_fields(node):
                next_parents, next_kind = (
                    (names, child_kind) if field == "body" else (parents, kind)
                )
                if isinstance(value, list):
                    for child in value:
                        if isinstance(child, ast.AST):
                            visit(child, next_parents, next_kind)
                elif isinstance(value, ast.AST):
                    visit(value, next_parents, next_kind)
            return
        if isinstance(node, ast.TypeAlias):
            child_names(node.name.id, parents, kind)
            for parameter in node.type_params:
                child_names(parameter.name, parents, kind)
        for child in ast.iter_child_nodes(node):
            visit(child, parents, kind)

    annotation_helpers({""}, "module")
    visit(tree, {""}, "module")
    return values
