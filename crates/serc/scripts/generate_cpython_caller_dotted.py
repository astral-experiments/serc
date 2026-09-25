"""Derive a scoped dotted-name inventory for the fixed CPython caller.

Run with the exact admitted immutable target runtime. Selected Python sources
are compiled, never executed. The resulting companion binds the existing
positive profile and runtime; it does not redefine their membership guarantees.
A reviewed target scope is required before absence can be claimed, and the
independent reader admission pins remain necessary even for complete scopes.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import posixpath
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import caller_dotted_metadata as lexical_metadata
import generate_cpython_caller as caller
import generate_cpython_caller_variants as variants
import generate_cpython_target as target

METADATA = Path(__file__).resolve().parent / "target_metadata"
PROFILE_PINS = {
    "3.13": (
        "3_13_15",
        "41b6c006a0fcdf27ef6c09728a0ffdc55e20514d7bba4aed8df27b677f78376e",
        "a244b184e89f34623244bddfa6ff7a03791eb9487eda850b2b8db83417dfedd6",
    ),
    "3.14": (
        "3_14_5",
        "5a7e6ca0176ab8e07bae6bde1d54226439462b6e82493e110f4863ce2bbf45e0",
        "a5f5312cc24c3329ef130339538c5ae2ad63fd878bf4743940a692637b6d1e68",
    ),
    "3.15": (
        "3_15_0rc2",
        "3580dff92980e02edea1334fd6fa7815bc2c7924ec79466a4389815a5e149a0a",
        "105ede4e32968da99515ddefebd37c838784d91100ceba87c3c81886dbd455e6",
    ),
}
# Completion requires a separate exact-target source-closure review. Merely
# generating or comparing candidate lists does not authorize an entry here.
REVIEWED_TARGETS: frozenset[str] = frozenset({"3.13", "3.14", "3.15"})

DOTTED = re.compile(
    r"[A-Za-z_][A-Za-z_0-9]*(?:\.(?:[A-Za-z_][A-Za-z_0-9]*|<locals>))+\Z"
)
STARTUP_ROOTS = (
    "Modules/gcmodule.c",
    "Modules/atexitmodule.c",
    "Modules/faulthandler.c",
    "Modules/_localemodule.c",
    "Modules/_tracemalloc.c",
    "Modules/_warnings.c",
    "Modules/main.c",
    "Modules/getpath.c",
    "Modules/getbuildinfo.c",
    "Modules/config.c.in",
    "Tools/build/freeze_modules.py",
)
COMMON_NATIVE = (
    "_operator",
    "errno",
    "_abc",
    "_stat",
    "_weakref",
    "_io",
    "posix",
    "_codecs",
    "_thread",
    "time",
    "_signal",
)
VERSION_NATIVE = {
    "3.13": ("_sre", "_functools", "_collections", "itertools", "pwd", "grp"),
    "3.14": (
        "_types",
        "_sre",
        "_functools",
        "_collections",
        "itertools",
        "pwd",
        "grp",
        "fcntl",
    ),
    "3.15": ("_types", "fcntl", "itertools", "pwd", "grp"),
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def c_literals(text):
    """Include individual and every adjacent C string group, across comments."""
    previous_end = 0
    adjacent = []
    values = set()
    for token in caller.C_TOKEN.finditer(text):
        between = text[previous_end : token.start()].replace("\\\n", "")
        if (between.strip() or token.lastgroup == "char") and adjacent:
            values.add("".join(adjacent))
            adjacent = []
        if token.lastgroup == "string":
            value = caller.decode_c_string(token[0])
            values.add(value)
            adjacent.append(value)
        previous_end = token.end()
    if adjacent:
        values.add("".join(adjacent))
    return values


def generate(source: Path, build_prefix: Path, version: str) -> dict:
    """Compile an exact source inventory under its admitted native compiler."""
    target.select_target_version(version)
    target.verify_checkout(source)
    if version == "3.14":
        frozen, ordinary = caller.FROZEN, caller.SOURCE_MODULES
    else:
        frozen, ordinary = variants.module_maps(source, version)
    expected = (
        (*target.TARGET_VERSION, "candidate", 2)
        if version == "3.15"
        else (*target.TARGET_VERSION, "final", 0)
    )
    if (
        sys.implementation.name != "cpython"
        or tuple(sys.version_info) != expected
        or sys.flags.optimize
    ):
        raise ValueError("generation requires the exact target interpreter")
    stem, profile_pin, runtime_pin = PROFILE_PINS[version]
    profile_bytes = (METADATA / f"cpython_{stem}_caller.json").read_bytes()
    admission_bytes = (
        METADATA / f"cpython_{stem}_caller_runtime_linux_x86_64.json"
    ).read_bytes()
    if digest(profile_bytes) != profile_pin or digest(admission_bytes) != runtime_pin:
        raise ValueError("initial profile or runtime differs from reviewed inputs")
    profile = json.loads(profile_bytes)
    admission = json.loads(admission_bytes)
    if (
        profile["target"]["commit"] != target.TARGET_COMMIT
        or admission["build"]["source_commit"] != target.TARGET_COMMIT
    ):
        raise ValueError("profile/runtime target source differs")
    if profile["oracle"]["script_sha256"] != caller.ORACLE_SHA256:
        raise ValueError("initial profile binds a different caller")
    compiler_sha = variants.verify_generation_runtime(admission)
    admitted_files = {entry["path"]: entry for entry in admission["files"]}
    tracked = caller.repository_paths(source)
    sources = {}
    inputs = {}
    candidates = {}
    recipes = []

    def read(path):
        if path not in sources:
            sources[path] = target.read_committed_file(source, path)
            inputs[path] = digest(sources[path].encode())
        return sources[path]

    def add(values, kind):
        for value in values:
            if DOTTED.fullmatch(value):
                candidates.setdefault(value, set()).add(kind)

    def add_literals(text, kind):
        values = c_literals(text)
        add(values, kind)
        # Deliberate overapproximation of module prefixes derived from tp_name.
        add(
            (value.rsplit(".", 1)[0] for value in values if "." in value),
            kind + ":type-prefix-overapprox",
        )

    native_modules = {
        name: caller.NATIVE_MODULES[name]
        for name in (*COMMON_NATIVE, *VERSION_NATIVE[version])
        if caller.NATIVE_MODULES[name] in tracked
    }
    missing_native_roots = {
        name: caller.NATIVE_MODULES[name]
        for name in (*COMMON_NATIVE, *VERSION_NATIVE[version])
        if caller.NATIVE_MODULES[name] not in tracked
    }
    groups = ["Modules/_io/"] + (
        ["Modules/_sre/"] if version in {"3.13", "3.14"} else []
    )
    roots = {
        path
        for path in tracked
        if path.split("/")[0] in {"Objects", "Python", "Parser", "Include"}
        and Path(path).suffix in {".c", ".h", ".inc"}
    }
    roots.update(native_modules.values())
    roots.update(path for path in STARTUP_ROOTS if path in tracked)
    roots.update(
        path
        for path in tracked
        if any(path.startswith(group) for group in groups)
        and Path(path).suffix in {".c", ".h", ".inc"}
    )
    # Every root is installed before the include closure starts.
    native = set(roots)
    pending = sorted(roots)
    missing_includes = {}
    while pending:
        path = pending.pop()
        for name in re.findall(r'^\s*#\s*include\s*"([^"]+)"', read(path), re.M):
            choices = [
                posixpath.normpath(str(Path(path).parent / name)),
                "Include/" + name,
                "Include/internal/" + name,
                "Include/internal/mimalloc/" + name,
            ]
            found = next((choice for choice in choices if choice in tracked), None)
            if found is None:
                missing_includes.setdefault(name, set()).add(path)
            elif found not in native:
                native.add(found)
                pending.append(found)
    for path in sorted(native):
        add_literals(read(path), "native-literal:" + path)
        if path.endswith(".py"):
            for node in ast.walk(ast.parse(read(path))):
                if isinstance(node, ast.Constant) and isinstance(node.value, str):
                    add([node.value], "generator-python-literal:" + path)

    posix_source = read("Modules/posixmodule.c")
    suffixes = sorted(
        set(re.findall(r'\bMODNAME\s+"(\.[A-Za-z_][A-Za-z_0-9]*)"', posix_source))
    )
    assert suffixes == [
        ".DirEntry",
        ".ScandirIterator",
        ".sched_param",
        ".times_result",
        ".uname_result",
        ".waitid_result",
    ]
    assert re.search(r'#\s*define\s+MODNAME\s+"posix"', posix_source)
    add(
        (prefix + suffix for prefix in ("posix", "nt") for suffix in suffixes),
        "MODNAME-finite-type-recipe",
    )
    recipes.append(
        {
            "kind": "native-MODNAME-type-concatenation-overapprox",
            "source": "Modules/posixmodule.c",
            "platform_branch": "Both finite alternatives conservatively include Linux/POSIX.",
            "prefixes": ["posix", "nt"],
            "suffixes": suffixes,
        }
    )

    build_inputs = []
    build_missing = []
    relatives = [
        f"include/python{version}/pyconfig.h",
        f"lib/python{version}/config-{version}-x86_64-linux-gnu/config.c",
        f"lib/python{version}/config-{version}-x86_64-linux-gnu/Makefile",
        f"lib/python{version}/_sysconfigdata__linux_x86_64-linux-gnu.py",
    ]
    for relative in relatives:
        path = build_prefix / relative
        if not path.is_file():
            build_missing.append(relative)
            continue
        data = path.read_bytes()
        fingerprint = digest(data)
        if relative in admitted_files:
            assert fingerprint == admitted_files[relative]["sha256"]
        inputs["runtime:" + relative] = fingerprint
        build_inputs.append(
            {
                "path": relative,
                "sha256": fingerprint,
                "bound_by_runtime_manifest": relative in admitted_files,
            }
        )
        text = data.decode()
        add_literals(text, "build-literal:" + relative)
        if path.suffix == ".py":
            # Parse data; never import or execute sysconfig module contents.
            tree = ast.parse(text)
            for node in ast.walk(tree):
                if isinstance(node, ast.Constant) and isinstance(node.value, str):
                    add([node.value], "build-python-literal:" + relative)

    python_sources = {**frozen, **ordinary, "getpath": "Modules/getpath.py"}
    assert {"zipimport", "linecache"}.issubset(python_sources)
    for module, path in python_sources.items():
        text = read(path)
        code = compile(
            text, "<frozen " + module + ">", "exec", dont_inherit=True, optimize=0
        )
        add(caller.code_facts(code)[1], "compiled-source:" + path)
        tree = ast.parse(text)
        add(lexical_metadata.lexical_qualnames(tree), "lexical-scope-metadata:" + path)
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str):
                add([node.value], "ast-literal:" + path)
            names = []
            if isinstance(node, ast.Import):
                names = [alias.name for alias in node.names]
            elif isinstance(node, ast.ImportFrom) and node.module:
                names = [node.module]
            for name in names:
                parts = name.split(".")
                add(
                    (".".join(parts[:count]) for count in range(2, len(parts) + 1)),
                    "parser-import-prefix:" + path,
                )

    if version in {"3.13", "3.14"}:
        tree = ast.parse(read("Lib/functools.py"))
        assignments = [
            node.value
            for node in tree.body
            if isinstance(node, ast.Assign)
            and any(
                isinstance(binding, ast.Name) and binding.id == "_CacheInfo"
                for binding in node.targets
            )
        ]
        assert len(assignments) == 1
        call = assignments[0]
        assert (
            isinstance(call, ast.Call)
            and isinstance(call.func, ast.Name)
            and call.func.id == "namedtuple"
        )
        typename = ast.literal_eval(call.args[0])
        fields = ast.literal_eval(call.args[1])
        assert typename == "CacheInfo" and fields == [
            "hits",
            "misses",
            "maxsize",
            "currsize",
        ]
        collection_source = read("Lib/collections/__init__.py")
        assert (
            "code = f'lambda _cls, {arg_list}: _tuple_new(_cls, ({arg_list}))'"
            in collection_source
        )
        assert (
            "method.__qualname__ = f'{typename}.{method.__name__}'" in collection_source
        )
        arg_list = ", ".join(fields)
        template = f"lambda _cls, {arg_list}: _tuple_new(_cls, ({arg_list}))"
        add(
            caller.code_facts(
                compile(template, "<string>", "eval", dont_inherit=True, optimize=0)
            )[1],
            "CacheInfo-eval-code",
        )
        methods = [
            "__new__",
            "_make",
            "_replace",
            "__repr__",
            "_asdict",
            "__getnewargs__",
        ]
        add(
            (typename + "." + method for method in methods),
            "CacheInfo-method-metadata-overapprox",
        )
        recipes.append(
            {
                "kind": "fixed-namedtuple",
                "source": "Lib/functools.py:_CacheInfo",
                "factory": "Lib/collections/__init__.py:namedtuple",
                "typename": typename,
                "fields": fields,
                "eval_source": template,
                "method_names": methods,
            }
        )
        if version == "3.13":
            recipes.append(
                {
                    "kind": "fixed-dotless-methodcaller",
                    "source": "Lib/glob.py:_Globber",
                    "arguments": [["lstat"], ["joinpath", ""]],
                }
            )
    else:
        recipes.append(
            {
                "kind": "no-eager-CacheInfo-or-glob-methodcaller",
                "reason": "The fixed caller does not force the declared lazy imports.",
            }
        )

    comparator = (target.CRATE_ROOT / "examples/compare_cpython_corpus.rs").read_text()
    oracle = re.search(r'const ORACLE: &str = r#"(.*?)"#;', comparator, re.S)[1]
    assert digest(oracle.encode()) == caller.ORACLE_SHA256
    add(
        caller.code_facts(
            compile(oracle, "<string>", "exec", dont_inherit=True, optimize=0)
        )[1],
        "oracle-code",
    )
    add(
        lexical_metadata.lexical_qualnames(ast.parse(oracle)),
        "oracle-lexical-scope-metadata",
    )
    # Known retained positives always dominate scoped absence. Their inclusion
    # here also makes that relationship explicit in the generated inventory.
    add(profile["state"]["retained_strings"], "initial-retained-positive")
    recipes.extend(
        [
            {
                "kind": "exact-source-and-native-inventory",
                "python_modules": dict(sorted(python_sources.items())),
                "optional_python_modules_included": ["zipimport", "linecache"],
                "native_modules": native_modules,
                "native_unit_groups": groups,
                "native_roots": sorted(roots),
                "native_include_closure": sorted(native),
                "missing_selected_native_roots": missing_native_roots,
                "unresolved_quoted_includes": {
                    name: sorted(paths)
                    for name, paths in sorted(missing_includes.items())
                },
            },
            {
                "kind": "runtime-build-inputs",
                "inputs": build_inputs,
                "missing": build_missing,
            },
            {
                "kind": "generator-dependencies",
                "sha256": {
                    path.name: digest(path.read_bytes())
                    for path in [
                        Path(caller.__file__),
                        Path(caller.bootstrap.__file__),
                        Path(caller.caller_source_owners.__file__),
                        Path(variants.__file__),
                        Path(target.__file__),
                        METADATA / "caller_dotted_proof.md",
                    ]
                },
            },
            {
                "kind": "lexical-and-final-code-metadata-overapproximation",
                "source_anchors": [
                    "Python/compile.c:compiler_set_qualname",
                    "Python/flowgraph.c:remove_unused_consts",
                ],
                "global_reset_alternatives": True,
                "annotation_helper_alternatives": True,
            },
            {
                "kind": "scoped-boundary",
                "phase": "before-target-source-decoding",
                "excludes_later_effects": [
                    "target source codecs",
                    "target compilation warnings",
                ],
                "filename_components": "The reader must add any target-specific caller filename interns before applying absence.",
            },
        ]
    )
    unresolved = (
        []
        if version in REVIEWED_TARGETS
        else [
            {
                "kind": "source-closure-review-pending",
                "detail": "The candidate inventory alone does not establish absence.",
            },
            {
                "kind": "build-and-dynamic-recipe-review",
                "detail": "Generated headers, linked native initialization and dynamic name construction need exact-target source review.",
            },
        ]
    )
    return {
        "schema_version": 1,
        "kind": "standard-caller-scoped-absence",
        "target": profile["target"],
        "initial_profile_sha256": digest(profile_bytes),
        "runtime_admission_sha256": digest(admission_bytes),
        "oracle_sha256": caller.ORACLE_SHA256,
        "compiler_executable_sha256": compiler_sha,
        "generator_sha256": digest(Path(__file__).read_bytes()),
        "lexical_helper_sha256": digest(Path(lexical_metadata.__file__).read_bytes()),
        "source_inputs": dict(sorted(inputs.items())),
        "domain": "ascii-dotted-identifiers-and-locals",
        "phase": "before-target-source-decoding",
        "scope_complete": version in REVIEWED_TARGETS,
        "possible_names": sorted(candidates),
        "coverage": {"recipes": recipes, "unresolved": unresolved},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=sorted(PROFILE_PINS), required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--build-prefix", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = generate(args.source, args.build_prefix, args.target)
    text = json.dumps(result, indent=2, ensure_ascii=True) + "\n"
    if args.check:
        if args.output.read_text() != text:
            raise ValueError("generated scoped inventory differs")
    else:
        args.output.write_text(text)
    print(
        json.dumps(
            {
                "target": args.target,
                "possible_names": len(result["possible_names"]),
                "scope_complete": result["scope_complete"],
            }
        )
    )


if __name__ == "__main__":
    main()
