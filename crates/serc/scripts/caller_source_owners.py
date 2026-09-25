"""Conservative retained-code owners for the fixed CPython caller import lane.

This does not execute imports or the supplied code. The caller must supply code
compiled from exactly ``source`` with the pinned CPython, and must establish a
successful, ordinary import with the audited dependencies. Only ASCII name-like
interns are returned. An unresolved item never contributes strings.

Unknown source uses a small static subset. The exact 3.14.5 sources below also
have a reviewed initialization-mutation contract: their eager calls do not
replace otherwise surviving function code. Source digests bind that exception;
module names alone do not. Conditional bindings and accelerator replacements
remain excluded even for these sources. Annotation helpers need a separate
owner proof and are deliberately omitted.
"""

from __future__ import annotations

import ast
import hashlib
from types import CodeType

# CPython v3.14.5, 5607950ef232dad16d75c0cf53101d9649d89115. Source facts only.
_AUDITED_SOURCES = {
    "pathlib": "042a4d0a5e1ef66778daa86dc269af205bd42d06e793e53d8b99dff4c45928ca",
    "pathlib._os": "61e844988e54bdf47c0b11ad2b3fd961f058bb370793428f4c6868bd40cab6f2",
    "glob": "e3c82e6f0a1fe9442d4c5625e68cb206228408f0342326f6f5c8f59112091b59",
    "fnmatch": "ce582bc266922e4c682e2a85d72095124430a6bf58716c6f2537103480eae742",
    "re": "741a9de729ed8207bfa19db990f8826f1bf3661f33d0970a80c08cd1338ebc35",
    "re._compiler": "d49f30cf9a1dbae33b200ed8befd9d0ce3ac612783a10ac35196536f98923e91",
    "re._parser": "e57bd194a2d42398355ae7c1ccc2ddfb78421dd431eb81e3809dbe8ca9057dc4",
    "re._constants": "42253b3181b81aad6c46392f44a0ab26dcfa31feea411296f43ba16616a1ab0b",
    "re._casefix": "1b12d9136f23db6c3f6f26053fefc15ca964b886838c7b9c1fabf8d2efc1e5c8",
    "enum": "982843c2dd6dbec992bfed44abe9ef5464be81876a180d977ccd1a69e26ec16d",
    "functools": "046ec9660146db48862c7b22c78e2ef3818eaed6920e5cbf35f551ef09fec6cd",
    "contextlib": "c1e0d67b2007de11ae93cd36cf6faf38d9ab32656a832d592a49325eec579f96",
    "collections": "dbd563752a57c6633b7566b1e9c95bc8ea81df1a1d213b23f61c5335424e5af5",
    "types": "fcfdd1c5ae56a9c35f207743d774e236da0153bf78e6e4629193a8e0e9438a6b",
    "operator": "a9f9910965c31f841caad0447ee47299ac539a371422aefe200c86c60f3b4697",
    "reprlib": "b04872e10d76252e44eae50cc785eb0fe478491afc4d7f0825a93f82b2eaba5f",
    "keyword": "18c2be738c04ad20ad375f6a71db34b3823c7f40b0340f5294d0e89f3c9b093b",
    "copyreg": "6376eb5722806396f5842997ac18add369ea9ac3ff4fcfe1460c41a088cac425",
    "os": "1494f3b93c6714d7a511963586f2b392dc92f3181c0d1ead82c621a4b0aad969",
    "io": "1b75584d4efcc612dc0db2ae98619408276585a106f0544879edb5e25b55430a",
    "abc": "e558702a95cdce3febd289da021715d2b92bc43995b8a1bc58dfa1c3d8010287",
    "stat": "07217986d9b2172b509dab3578d18ac05ba768b418e1f1e8c90e38a8f7c2b3c6",
    "ntpath": "7c5c992b1b52f97d7dc73ad09ea8b97917475e7e4474563379e71b03015f04ca",
    "posixpath": "8a5bc2b6674e76efe0f507b873a41d746beba466a191153ed18a7580ef6e08e1",
    "genericpath": "cdac5a68dd738051e0d66fc81d5947d6a1776248fc6fe862253477608d870ec4",
    "_collections_abc": "50beebb1f309d83fd3af45f0e4c98c9dbfe0edd2f40edc8dc79e5cf0072cfbea",
}
_FUNCTIONS = (ast.FunctionDef, ast.AsyncFunctionDef)
_BUILTINS = {
    name: f"builtins.{name}"
    for name in (
        "property",
        "classmethod",
        "staticmethod",
        "object",
    )
}
_WRAPPERS = {
    "builtins.property",
    "builtins.classmethod",
    "builtins.staticmethod",
    "abc.abstractmethod",
    "functools.lru_cache",
    "reprlib.recursive_repr",
}
_FACTORIES = {"functools.lru_cache", "reprlib.recursive_repr"}
_SYNTHETIC_BINDINGS = {
    "__annotate__",
    "__annotate_func__",
    "__static_attributes__",
    "__classcell__",
    "__classdictcell__",
}


def _mangle(name: str, class_name: str | None) -> str:
    if class_name and name.startswith("__") and not name.endswith("__"):
        stripped = class_name.lstrip("_")
        if stripped:
            return "_" + stripped + name
    return name


def _code_strings(code: CodeType) -> set[str]:
    """Walk only strong code/constant edges below an established function owner."""
    result = set()

    def add(value):
        if isinstance(value, str):
            if value.isascii() and all(c.isalnum() or c == "_" for c in value):
                result.add(value)
        elif isinstance(value, CodeType):
            for name in (
                *value.co_names,
                *value.co_varnames,
                *value.co_cellvars,
                *value.co_freevars,
                value.co_name,
                value.co_qualname,
            ):
                add(name)
            for constant in value.co_consts:
                add(constant)
        elif isinstance(value, (tuple, frozenset)):
            for item in value:
                add(item)

    add(code)
    return result


def _code_names(code: CodeType) -> set[str]:
    """Report retained metadata; interning policy is the target caller's concern."""
    names = {code.co_name, code.co_qualname}
    for constant in code.co_consts:
        if isinstance(constant, CodeType):
            names.update(_code_names(constant))
    return names


class _Writes(ast.NodeVisitor):
    """Possible writes in this executed scope, without entering function bodies."""

    def __init__(self):
        self.names = set()
        self.mutation_roots = set()
        self.star = False
        self.definitions = []

    def visit_Name(self, node):
        if isinstance(node.ctx, (ast.Store, ast.Del)):
            self.names.add(node.id)

    def visit_Attribute(self, node):
        if isinstance(node.ctx, (ast.Store, ast.Del)):
            root = node.value
            while isinstance(root, (ast.Attribute, ast.Subscript)):
                root = root.value
            if isinstance(root, ast.Name):
                self.names.add(root.id)
                self.mutation_roots.add(root.id)
        self.generic_visit(node)

    visit_Subscript = visit_Attribute

    def visit_FunctionDef(self, node):
        self.names.add(node.name)
        self.definitions.append(node)
        for expression in (
            *node.args.defaults,
            *node.args.kw_defaults,
            *node.decorator_list,
        ):
            if expression is not None:
                self.visit(expression)

    visit_AsyncFunctionDef = visit_FunctionDef

    def visit_ClassDef(self, node):
        self.names.add(node.name)
        self.definitions.append(node)
        for expression in (
            *node.bases,
            *node.decorator_list,
            *(keyword.value for keyword in node.keywords),
        ):
            self.visit(expression)

    def visit_AnnAssign(self, node):
        if node.value is not None:
            self.visit(node.target)
            self.visit(node.value)

    def visit_Import(self, node):
        self.names.update(
            alias.asname or alias.name.split(".")[0] for alias in node.names
        )

    def visit_ImportFrom(self, node):
        self.star |= any(alias.name == "*" for alias in node.names)
        self.names.update(alias.asname or alias.name for alias in node.names)

    def visit_ExceptHandler(self, node):
        if node.name:
            self.names.add(node.name)
        self.generic_visit(node)

    def visit_MatchAs(self, node):
        if node.name:
            self.names.add(node.name)
        self.generic_visit(node)

    visit_MatchStar = visit_MatchAs

    def visit_MatchMapping(self, node):
        if node.rest:
            self.names.add(node.rest)
        self.generic_visit(node)


def derive_retained_code(source: str, code: CodeType, module_name: str) -> dict:
    """Return proven retained ASCII interns, their owners, and excluded candidates.

    ``owners`` has one entry per retained source function/accessor, including its
    complete recursively owned string set. Module and class suite constants are
    never included merely because the module/class survives.
    """
    tree = ast.parse(source)
    audited = (
        _AUDITED_SOURCES.get(module_name) == hashlib.sha256(source.encode()).hexdigest()
    )
    unresolved = []
    tainted = False

    def reject(node, reason):
        unresolved.append(
            {
                "name": getattr(node, "name", module_name),
                "line": getattr(node, "lineno", 0),
                "reason": reason,
            }
        )

    def effect(node):
        nonlocal tainted
        if audited or node is None:
            return
        # Calls in function bodies are not run by defining their owner.
        if isinstance(node, _FUNCTIONS):
            for value in (*node.args.defaults, *node.args.kw_defaults):
                effect(value)
            return
        if isinstance(node, ast.Call):
            reject(node, "unproved eager call may mutate code owners")
            tainted = True
        if isinstance(node, (ast.Global, ast.Nonlocal)):
            reject(node, "cross-scope binding effects are unsupported")
            tainted = True
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            modules = (
                [item.name for item in node.names]
                if isinstance(node, ast.Import)
                else [node.module]
            )
            if any(
                name not in {"abc", "functools", "reprlib", "builtins"}
                for name in modules
            ):
                reject(node, "import initialization has no mutation audit")
                tainted = True
        for child in ast.iter_child_nodes(node):
            effect(child)

    def child_code(parent, node):
        if getattr(node, "type_params", ()):
            reject(node, "generic scope requires a separate owner mapping")
            return None
        line = min([node.lineno] + [d.lineno for d in node.decorator_list])
        matches = [
            item
            for item in parent.co_consts
            if isinstance(item, CodeType)
            and item.co_name == node.name
            and item.co_firstlineno == line
        ]
        if len(matches) != 1:
            reject(node, "no unique immediate child code object")
            return None
        return matches[0]

    def resolve(node, aliases):
        if isinstance(node, ast.Name):
            return aliases.get(node.id)
        if isinstance(node, ast.Attribute):
            base = resolve(node.value, aliases)
            return f"{base}.{node.attr}" if base else None
        return None

    def body(
        statements, parent, scope, inherited_aliases, outer_classes, class_name=None
    ):
        nonlocal tainted
        bindings = {}
        shadowed = set()
        aliases = dict(inherited_aliases)

        def kill(name, node):
            name = _mangle(name, class_name)
            shadowed.add(name)
            if name in bindings:
                reject(node, f"binding replaces or deletes owner {scope}.{name}")
                del bindings[name]
            aliases.pop(name, None)

        for node in statements:
            if isinstance(node, _FUNCTIONS):
                effect(node)
                eager_writes = _Writes()
                for expression in (
                    *node.args.defaults,
                    *node.args.kw_defaults,
                    *node.decorator_list,
                ):
                    if expression is not None:
                        eager_writes.visit(expression)
                for name in eager_writes.names:
                    kill(name, node)
                key = _mangle(node.name, class_name)
                function = child_code(parent, node)
                record = (function, f"{scope}.{key}", node.lineno)
                candidate = {"kind": "function", "parts": {"function": record}}
                for decorator in reversed(node.decorator_list):
                    if (
                        isinstance(decorator, ast.Attribute)
                        and isinstance(decorator.value, ast.Name)
                        and decorator.attr in {"getter", "setter", "deleter"}
                    ):
                        previous = bindings.get(_mangle(decorator.value.id, class_name))
                        if previous and previous["kind"] == "property":
                            parts = dict(previous["parts"])
                            parts[decorator.attr] = record
                            candidate = {"kind": "property", "parts": parts}
                            continue
                    target = (
                        decorator.func if isinstance(decorator, ast.Call) else decorator
                    )
                    identity = resolve(target, aliases)
                    if identity not in _WRAPPERS or (
                        isinstance(decorator, ast.Call) and identity not in _FACTORIES
                    ):
                        reject(node, "unproved decorator retention")
                        candidate = None
                        if not audited:
                            tainted = True
                        break
                    if isinstance(decorator, ast.Call):
                        for argument in (
                            *decorator.args,
                            *(kw.value for kw in decorator.keywords),
                        ):
                            effect(argument)
                    if identity == "builtins.property":
                        candidate = {"kind": "property", "parts": {"getter": record}}
                kill(node.name, node)
                if (
                    function is not None
                    and candidate is not None
                    and key not in _SYNTHETIC_BINDINGS
                ):
                    bindings[key] = candidate
                elif key in _SYNTHETIC_BINDINGS:
                    reject(node, "compiler-generated binding may replace this owner")
                continue

            if isinstance(node, ast.ClassDef):
                key = _mangle(node.name, class_name)
                known_classes = {
                    name: value
                    for name, value in outer_classes.items()
                    if name not in shadowed
                }
                known_classes.update(bindings)
                plain = not node.decorator_list and not node.keywords
                for base in node.bases:
                    inherited = (
                        known_classes.get(base.id)
                        if isinstance(base, ast.Name)
                        else None
                    )
                    plain &= resolve(base, aliases) == "builtins.object" or bool(
                        inherited
                        and inherited["kind"] == "class"
                        and inherited["subclass_safe"]
                    )
                nested_code = child_code(parent, node) if plain else None
                kill(node.name, node)
                if nested_code is None:
                    reject(
                        node,
                        "class metaclass, base, or decorator ownership is unproved",
                    )
                    if not audited:
                        tainted = True
                    continue
                members = body(
                    node.body,
                    nested_code,
                    f"{scope}.{key}",
                    aliases,
                    known_classes,
                    node.name,
                )
                class_writes = _Writes()
                for statement in node.body:
                    class_writes.visit(statement)
                bindings[key] = {
                    "kind": "class",
                    "members": members,
                    "subclass_safe": "__init_subclass__" not in class_writes.names,
                }
                continue

            if isinstance(node, (ast.Global, ast.Nonlocal)):
                reject(node, "cross-scope binding effects are unsupported")
                tainted = True
                continue
            effect(node)
            writes = _Writes()
            writes.visit(node)
            if not audited and any(
                _mangle(name, class_name) not in bindings
                for name in writes.mutation_roots
            ):
                reject(node, "mutation of an unproved or enclosing owner")
                tainted = True
            if writes.star:
                for name in list(bindings):
                    kill(name, node)
                aliases.clear()
                if audited:
                    aliases.update(_BUILTINS)
                reject(node, "star import may replace preceding owners")
            for name in writes.names:
                kill(name, node)
            for definition in writes.definitions:
                reject(definition, "conditional owner is not proved selected")
            if isinstance(node, ast.Import):
                for item in node.names:
                    aliases[item.asname or item.name.split(".")[0]] = (
                        item.name if item.asname else item.name.split(".")[0]
                    )
            elif isinstance(node, ast.ImportFrom) and node.level == 0:
                for item in node.names:
                    if item.name != "*":
                        aliases[item.asname or item.name] = f"{node.module}.{item.name}"
        return bindings

    bindings = body(tree.body, code, module_name, _BUILTINS, {})
    owners = []

    def collect(values):
        for binding in values.values():
            if binding["kind"] == "class":
                collect(binding["members"])
            else:
                for function, owner, line in binding["parts"].values():
                    owners.append(
                        {
                            "owner": owner,
                            "line": line,
                            "kind": binding["kind"],
                            "strings": sorted(_code_strings(function)),
                            "code_names": sorted(_code_names(function)),
                        }
                    )

    if not tainted:
        collect(bindings)
    owners.sort(key=lambda owner: (owner["owner"], owner["line"]))
    strings = sorted({value for owner in owners for value in owner["strings"]})
    unresolved.sort(key=lambda item: (item["line"], item["name"], item["reason"]))
    return {"strings": strings, "owners": owners, "unresolved": unresolved}
