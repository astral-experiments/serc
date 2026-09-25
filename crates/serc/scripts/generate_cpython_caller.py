#!/usr/bin/env python3
"""Derive source facts for the pinned, isolated pathlib comparison caller.

Generation compiles exact-tag module sources but never executes them or observes
an interpreter's intern table. A generated candidate inventory is not admission:
the complete source proof and separate runtime manifest need independent review.

The first admitted contract is guaranteed-retained-partial. Unlisted strings
remain unknown, and no candidate inventory is used to prove absence. Its positive
families are: fixed native startup owners from the independently audited exact
build; source-defined static/AST/single-character interns; code identifiers;
mandatory interned metadata and word constants loaded from frozen marshal data;
surviving source function code and its recursively owned constants; the retained
codec-alias dictionary; the fixed CacheInfo namedtuple; and the suspended oracle
frame. Source-loaded module and class suites are not treated as surviving code.

Objects/codeobject.c constructs immortal identifier entries and mortal ASCII
constant/metadata entries. Programs/_freeze_module.c and _freeze_module.py compile
at optimize=0, then Python/marshal.c reads their interned tags as immortal. For
ordinary source loads, caller_source_owners.py proves actual surviving functions
and retaining wrappers against exact module hashes. Retained co_filename values
expand only beneath the admitted runtime prefix. The source-only runtime's exact
binary and native libraries match the native startup owner evidence; this does
not import the cached-source snapshot's complete immortal table. Additional
native positives and absence coverage are deliberately left unclaimed.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from types import CodeType

sys.path.insert(0, str(Path(__file__).resolve().parent))
import caller_source_owners
import generate_cpython_bootstrap as bootstrap
import generate_cpython_target as target

ORACLE_SHA256 = "6812e016e7e871700001ad9f8f832c5998d553bc33c84afb1745dbd664a965fb"
STARTUP_SHA256 = "1c0a3051c44c5307327db0f331d259e65a7e648da1cb5b30a0b2f8d9ca059f02"
ASCII_WORD = re.compile(r"[A-Za-z0-9_]+\Z")
WORDS = re.compile(r"[A-Za-z0-9_]+")

# Eager imports of the fixed caller, including its guaranteed neutral bootstrap.
# The runtime admission separately verifies this exact frozen/source selection.
FROZEN = {
    "importlib._bootstrap": "Lib/importlib/_bootstrap.py",
    "importlib._bootstrap_external": "Lib/importlib/_bootstrap_external.py",
    "zipimport": "Lib/zipimport.py",
    "abc": "Lib/abc.py",
    "codecs": "Lib/codecs.py",
    "io": "Lib/io.py",
    "_collections_abc": "Lib/_collections_abc.py",
    "genericpath": "Lib/genericpath.py",
    "ntpath": "Lib/ntpath.py",
    "posixpath": "Lib/posixpath.py",
    "os": "Lib/os.py",
    "stat": "Lib/stat.py",
}
SOURCE_MODULES = {
    "encodings": "Lib/encodings/__init__.py",
    "encodings.aliases": "Lib/encodings/aliases.py",
    "encodings.utf_8": "Lib/encodings/utf_8.py",
    "linecache": "Lib/linecache.py",
    "pathlib": "Lib/pathlib/__init__.py",
    "pathlib._os": "Lib/pathlib/_os.py",
    "glob": "Lib/glob.py",
    "fnmatch": "Lib/fnmatch.py",
    "re": "Lib/re/__init__.py",
    "re._compiler": "Lib/re/_compiler.py",
    "re._parser": "Lib/re/_parser.py",
    "re._constants": "Lib/re/_constants.py",
    "re._casefix": "Lib/re/_casefix.py",
    "enum": "Lib/enum.py",
    "functools": "Lib/functools.py",
    "contextlib": "Lib/contextlib.py",
    "collections": "Lib/collections/__init__.py",
    "types": "Lib/types.py",
    "operator": "Lib/operator.py",
    "reprlib": "Lib/reprlib.py",
    "keyword": "Lib/keyword.py",
    "copyreg": "Lib/copyreg.py",
}
NATIVE_MODULES = {
    "_types": "Modules/_typesmodule.c",
    "_sre": "Modules/_sre/sre.c",
    "_functools": "Modules/_functoolsmodule.c",
    "_collections": "Modules/_collectionsmodule.c",
    "_operator": "Modules/_operator.c",
    "itertools": "Modules/itertoolsmodule.c",
    "errno": "Modules/errnomodule.c",
    "pwd": "Modules/pwdmodule.c",
    "grp": "Modules/grpmodule.c",
    "fcntl": "Modules/fcntlmodule.c",
    "_abc": "Modules/_abc.c",
    "_stat": "Modules/_stat.c",
    "_weakref": "Modules/_weakref.c",
    "_io": "Modules/_io/_iomodule.c",
    "posix": "Modules/posixmodule.c",
    "_codecs": "Modules/_codecsmodule.c",
    "_thread": "Modules/_threadmodule.c",
    "time": "Modules/timemodule.c",
    "_signal": "Modules/signalmodule.c",
}
GENERIC_SOURCES = (
    "Objects/object.c",
    "Objects/typeobject.c",
    "Objects/unicodeobject.c",
    "Objects/dictobject.c",
    "Objects/descrobject.c",
    "Objects/moduleobject.c",
    "Objects/structseq.c",
    "Objects/odictobject.c",
    "Objects/codeobject.c",
    "Objects/funcobject.c",
    "Python/modsupport.c",
    "Python/getargs.c",
    "Python/import.c",
    "Python/pylifecycle.c",
    "Python/sysmodule.c",
    "Python/pythonrun.c",
    "Python/codecs.c",
    "Python/symtable.c",
    "Python/compile.c",
    "Python/codegen.c",
    "Python/ast.c",
    "Python/Python-ast.c",
    "Python/marshal.c",
    "Python/bltinmodule.c",
    "Modules/_io/fileio.c",
    "Modules/_io/iobase.c",
    "Modules/_io/bufferedio.c",
    "Modules/_io/textio.c",
    "Modules/getpath.c",
    "Modules/main.c",
    "Parser/pegen.c",
    "Parser/string_parser.c",
    "Parser/action_helpers.c",
    "Include/internal/pycore_global_strings.h",
    "Tools/build/freeze_modules.py",
)
ENUM_PROGRESS_CLASSES = (
    "Enum",
    "ReprEnum",
    "IntEnum",
    "StrEnum",
    "FlagBoundary",
    "Flag",
    "IntFlag",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def strings_in(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, (tuple, frozenset)):
        for member in value:
            yield from strings_in(member)


def code_facts(code: CodeType) -> tuple[set[str], set[str]]:
    immortal = set()
    possible = set()
    for child in bootstrap.code_tree(code):
        immortal.update(child.co_names)
        immortal.update(child.co_varnames)
        immortal.update(child.co_cellvars)
        immortal.update(child.co_freevars)
        possible.update((child.co_name, child.co_qualname, child.co_filename))
        possible.update(strings_in(child.co_consts))
    possible.update(immortal)
    return immortal, possible


def frozen_interns(code: CodeType) -> set[str]:
    """Interned marshal tags immortalize frozen metadata and name-like literals."""
    retained = set()
    for child in bootstrap.code_tree(code):
        retained.update((child.co_filename, child.co_name, child.co_qualname))
        retained.update(bootstrap.identifier_constants(child.co_consts))
    return retained


def candidate_words(values) -> set[str]:
    words = {word for value in values for word in WORDS.findall(value)}
    # Fixed standard-library string builders use these case conversions. This
    # overapproximation is never a positive-membership claim.
    return words | {
        variant
        for word in words
        for variant in (word.lower(), word.upper(), word.title(), word.capitalize())
    }


C_TOKEN = re.compile(
    r"(?P<comment>/\*.*?\*/|//[^\n]*)"
    r"|(?P<char>(?:u8|[LuU])?\'(?:\\.|[^\'\\])*\')"
    r'|(?P<string>(?:u8|[LuU])?"(?:\\.|[^"\\])*")',
    re.S,
)


def decode_c_string(literal: str) -> str:
    """Decode C escapes, including C's unlimited-length hexadecimal escape."""
    literal = literal[literal.index('"') + 1 : -1]
    output = []
    index = 0
    escapes = {
        "a": "\a",
        "b": "\b",
        "f": "\f",
        "n": "\n",
        "r": "\r",
        "t": "\t",
        "v": "\v",
    }
    while index < len(literal):
        char = literal[index]
        index += 1
        if char != "\\":
            output.append(char)
            continue
        char = literal[index]
        index += 1
        if char == "\n":
            continue
        if char == "x":
            match = re.match(r"[0-9a-fA-F]+", literal[index:])
            if match is None:
                raise ValueError("invalid C hexadecimal escape")
            index += len(match[0])
            point = int(match[0], 16)
            if point > 0x10FFFF:
                raise ValueError("unbounded C escape cannot be a Unicode spelling")
            output.append(chr(point))
        elif char in "01234567":
            match = re.match(r"[0-7]{0,2}", literal[index:])
            assert match is not None
            index += len(match[0])
            output.append(chr(int(char + match[0], 8)))
        elif char in "uU":
            width = 4 if char == "u" else 8
            output.append(chr(int(literal[index : index + width], 16)))
            index += width
        else:
            output.append(escapes.get(char, char))
    return "".join(output)


def c_candidates(source: str) -> set[str]:
    """Include macro names and decoded adjacent literals, retaining false positives."""
    values = [source]
    previous_end = 0
    adjacent = []
    for token in C_TOKEN.finditer(source):
        between = source[previous_end : token.start()].replace("\\\n", "")
        if (between.strip() or token.lastgroup == "char") and adjacent:
            values.append("".join(adjacent))
            adjacent.clear()
        if token.lastgroup == "string":
            value = decode_c_string(token[0])
            values.append(value)
            adjacent.append(value)
        previous_end = token.end()
    if adjacent:
        values.append("".join(adjacent))
    return candidate_words(values)


def repository_paths(checkout: Path) -> set[str]:
    data = subprocess.check_output(
        [
            "git",
            "-C",
            str(checkout),
            "ls-tree",
            "-r",
            "--name-only",
            target.TARGET_COMMIT,
        ],
        text=True,
    )
    return set(data.splitlines())


def native_closure(checkout: Path, sources: dict[str, str]) -> list[str]:
    """Follow repository-owned quoted includes for the reviewed native families."""
    paths = repository_paths(checkout)
    pending = list(NATIVE_MODULES.values()) + list(GENERIC_SOURCES)
    native = set()
    while pending:
        path = pending.pop()
        if path in native:
            continue
        source = sources.setdefault(path, target.read_committed_file(checkout, path))
        native.add(path)
        for name in re.findall(r'^\s*#\s*include\s*"([^"]+)"', source, re.M):
            candidates = [
                str(Path(path).parent / name),
                f"Include/{name}",
                f"Include/internal/{name}",
            ]
            found = next(
                (candidate for candidate in candidates if candidate in paths), None
            )
            if found is not None:
                pending.append(found)
    return sorted(native)


def generate(
    checkout: Path, startup: Path, admissions: list[Path], output: Path
) -> dict:
    target.select_target_version("3.14")
    target.verify_checkout(checkout)
    if sys.implementation.name != "cpython" or tuple(sys.version_info) != (
        3,
        14,
        5,
        "final",
        0,
    ):
        raise ValueError("caller generation requires exact CPython 3.14.5")
    startup_bytes = startup.read_bytes()
    if digest(startup_bytes) != STARTUP_SHA256:
        raise ValueError("startup owner evidence differs from the reviewed artifact")
    base = json.loads(startup_bytes)
    if (base["target_tag"], base["target_commit"]) != (
        target.TARGET_TAG,
        target.TARGET_COMMIT,
    ):
        raise ValueError("startup evidence belongs to a different target")
    if (
        base["retention"]["proved_mortal_entries"]
        != base["retention"]["mortal_dynamic_entries"]
    ):
        raise ValueError("startup evidence has unproved mortal entries")
    if (
        digest(Path(sys.executable).read_bytes())
        != base["runtime_profile"]["executable_sha256"]
    ):
        raise ValueError(
            "source facts require the independently audited exact compiler executable"
        )

    sources: dict[str, str] = {}
    for path in (
        "Include/internal/pycore_global_strings.h",
        "Include/internal/pycore_magic_number.h",
        "Python/Python-ast.c",
        "Objects/codeobject.c",
    ):
        sources[path] = target.read_committed_file(checkout, path)
    if "_PyUnicode_InternImmortal(interp" not in sources["Objects/codeobject.c"]:
        raise ValueError("code identifier interning implementation changed")
    # The cached-source neutral snapshot is not a retained-positive list for a
    # source-only caller. Only its separately audited native roots transfer;
    # derive frozen and source-code facts independently below.
    retained = set(base["retention"]["mortal_owners"])
    retained.update(
        target.parse_preinterned_strings(
            sources["Include/internal/pycore_global_strings.h"]
        )
    )
    retained.update(
        target.parse_compiler_interned_strings(sources["Python/Python-ast.c"])
    )
    retained.update(chr(point) for point in range(256))
    retained.add("")
    possible = {
        word for word in {*retained, *base["strings"]} if ASCII_WORD.fullmatch(word)
    }
    owners = [
        {
            "kind": "same-binary-native-startup-roots",
            "startup_profile_sha256": digest(startup_bytes),
            "entries": len(base["retention"]["mortal_owners"]),
        }
    ]
    modules = []
    runtime_paths = []
    all_classes = set()
    private_names = set()

    for name, path in {**FROZEN, **SOURCE_MODULES}.items():
        source = target.read_committed_file(checkout, path)
        sources[path] = source
        filename = f"<frozen {name}>" if name in FROZEN else f"<stdlib>/{path[4:]}"
        code = compile(source, filename, "exec", dont_inherit=True, optimize=0)
        identifiers, code_strings = code_facts(code)
        retained.update(identifiers)
        if name in FROZEN:
            retained.update(frozen_interns(code))
        possible.update(candidate_words(code_strings))
        tree = ast.parse(source, filename)
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str):
                possible.update(candidate_words([node.value]))
            if isinstance(node, ast.Name):
                possible.add(node.id)
            if isinstance(node, ast.Attribute):
                possible.add(node.attr)
            if isinstance(node, ast.ClassDef):
                all_classes.add(node.name)
            if (
                isinstance(node, ast.Constant)
                and isinstance(node.value, str)
                and node.value.startswith("__")
                and not node.value.endswith("__")
            ):
                private_names.add(node.value)
        if name == "encodings.aliases":
            bindings = [
                node
                for node in tree.body
                if isinstance(node, ast.Assign)
                and any(
                    isinstance(target_node, ast.Name) and target_node.id == "aliases"
                    for target_node in node.targets
                )
            ]
            if len(bindings) != 1:
                raise ValueError(
                    "codec aliases no longer have one reviewed literal dictionary"
                )
            alias_dictionary = ast.literal_eval(bindings[0].value)
            if not isinstance(alias_dictionary, dict) or not all(
                isinstance(key, str) and isinstance(value, str)
                for key, value in alias_dictionary.items()
            ):
                raise ValueError("codec alias dictionary has nonliteral string entries")
            alias_interns = set(bootstrap.identifier_constants(alias_dictionary))
            retained.update(alias_interns)
            owners.append(
                {
                    "kind": "retained-module-literal-dictionary",
                    "module": name,
                    "binding": "aliases",
                    "strings": sorted(alias_interns),
                }
            )
        facts = caller_source_owners.derive_retained_code(source, code, name)
        retained.update(facts["strings"])
        for owner in facts["owners"]:
            retained.update(owner["code_names"])
        owners.extend({"module": name, **owner} for owner in facts["owners"])
        modules.append(
            {
                "name": name,
                "source_path": path,
                "selection": "frozen" if name in FROZEN else "source",
                "code_objects": len(tuple(bootstrap.code_tree(code))),
                "immortal_identifiers": len(identifiers),
            }
        )
        if name not in FROZEN and facts["owners"]:
            runtime_paths.append(f"lib/python3.14/{path[4:]}")

    # The fixed wrapper remains suspended throughout compilation and therefore
    # owns its own constants, unlike completed imported module/class suites.
    comparator = (target.CRATE_ROOT / "examples/compare_cpython_corpus.rs").read_text()
    oracle = re.search(r'const ORACLE: &str = r#"(.*?)"#;', comparator, re.S)
    if oracle is None or digest(oracle[1].encode()) != ORACLE_SHA256:
        raise ValueError("comparator oracle differs from the source-reviewed caller")
    wrapper = compile(oracle[1], "<string>", "exec", dont_inherit=True, optimize=0)
    identifiers, wrapper_strings = code_facts(wrapper)
    retained.update(identifiers)
    retained.update(bootstrap.identifier_constants(wrapper.co_consts))
    retained.update((wrapper.co_name, wrapper.co_qualname, wrapper.co_filename))
    possible.update(candidate_words(wrapper_strings))
    owners.append(
        {
            "kind": "suspended-oracle-frame",
            "code_name": wrapper.co_name,
            "script_sha256": ORACLE_SHA256,
        }
    )

    for path in native_closure(checkout, sources):
        possible.update(c_candidates(sources[path]))
    getpath = target.read_committed_file(checkout, "Modules/getpath.py")
    sources["Modules/getpath.py"] = getpath
    possible.update(candidate_words([getpath]))
    getpath_code = compile(
        getpath, "<frozen getpath>", "exec", dont_inherit=True, optimize=0
    )
    retained.update(code_facts(getpath_code)[0])
    retained.update(frozen_interns(getpath_code))
    for proof_path in (
        "Programs/_freeze_module.c",
        "Programs/_freeze_module.py",
        "Python/frozen.c",
        "Makefile.pre.in",
    ):
        sources[proof_path] = target.read_committed_file(checkout, proof_path)
    # Enum's deletion lookup can retain these names in type caches; lifetime is
    # collision-dependent, so these spellings remain candidates, never positives.
    progress = {f"_{name}__in_progress" for name in ENUM_PROGRESS_CLASSES}
    possible.update(progress)
    # Private slots go through _Py_Mangle before descriptor-name interning.
    mangled = {
        f"_{name.lstrip('_')}{member}"
        for name in all_classes
        for member in private_names
        if name.lstrip("_") and "." not in member
    }
    possible.update(mangled)
    # functools._CacheInfo owns fixed namedtuple fields and generated lambda.
    cache_bindings = [
        node
        for node in ast.parse(sources["Lib/functools.py"]).body
        if isinstance(node, ast.Assign)
        and any(
            isinstance(target_node, ast.Name) and target_node.id == "_CacheInfo"
            for target_node in node.targets
        )
    ]
    if len(cache_bindings) != 1 or not isinstance(cache_bindings[0].value, ast.Call):
        raise ValueError("functools CacheInfo has no unique reviewed constructor")
    cache_call = cache_bindings[0].value
    if (
        not isinstance(cache_call.func, ast.Name)
        or cache_call.func.id != "namedtuple"
        or len(cache_call.args) != 2
        or cache_call.keywords
    ):
        raise ValueError("functools CacheInfo constructor changed")
    cache_name = ast.literal_eval(cache_call.args[0])
    cache_fields = ast.literal_eval(cache_call.args[1])
    if cache_name != "CacheInfo" or cache_fields != [
        "hits",
        "misses",
        "maxsize",
        "currsize",
    ]:
        raise ValueError("namedtuple synthesis requires a new source audit")
    argument_list = ", ".join(cache_fields)
    cache_source = f"lambda _cls, {argument_list}: _tuple_new(_cls, ({argument_list}))"
    cache_code = compile(
        cache_source, "<string>", "eval", dont_inherit=True, optimize=0
    )
    cache_identifiers, cache_possible = code_facts(cache_code)
    retained.update(cache_identifiers)
    retained.update((cache_name, *cache_fields))
    retained.update(
        f"Alias for field number {index}" for index in range(len(cache_fields))
    )
    possible.update(candidate_words(cache_possible))
    possible.update(word for word in retained if ASCII_WORD.fullmatch(word))
    # Partial semantics never consumes candidate words as an absence proof.
    # Keep only the schema-required retained subset in the public artifact.
    possible = {word for word in retained if ASCII_WORD.fullmatch(word)}

    refs = []
    for admission in admissions:
        admitted = json.loads(admission.read_bytes())
        executable = next(
            entry
            for entry in admitted["files"]
            if entry["path"] == admitted["executable"]
        )
        if executable["sha256"] != base["runtime_profile"]["executable_sha256"]:
            raise ValueError(
                "native startup owners require the independently audited exact executable"
            )
        relative = admission.resolve().relative_to(output.parent.resolve())
        refs.append({"path": str(relative), "sha256": digest(admission.read_bytes())})
    magic = int(
        re.search(
            r"^#define\s+PYC_MAGIC_NUMBER\s+(\d+)$",
            sources["Include/internal/pycore_magic_number.h"],
            re.M,
        )[1]
    )
    return {
        "schema_version": 1,
        "kind": "standard-caller-source-proof",
        "target": {
            "implementation": "cpython",
            "version": "3.14.5",
            "tag": target.TARGET_TAG,
            "commit": target.TARGET_COMMIT,
            "magic_hex": (magic.to_bytes(2, "little") + b"\r\n").hex(),
        },
        "supported_compiler_input_modes": ["source-bytes", "utf8-text"],
        "oracle": {
            "script_sha256": ORACLE_SHA256,
            "arguments": ["-I", "-S", "-c"],
            "source_input": "bytes",
            "compile_mode": "exec",
            "dont_inherit": True,
            "optimize": 0,
        },
        "platform": "linux",
        "architecture": "x86_64",
        "gil_enabled": True,
        "generator_sha256": digest(Path(__file__).read_bytes()),
        "input_sha256": {
            path: digest(source.encode()) for path, source in sorted(sources.items())
        },
        "startup_profile_sha256": digest(startup_bytes),
        "state": {
            "semantics": "guaranteed-retained-partial",
            "positive_proof_complete": True,
            "retained_strings": sorted(retained),
            "retained_runtime_paths": sorted(runtime_paths),
            "possible_ascii_words": sorted(possible),
            "absence_domain": "nonempty-ascii-alnum-underscore",
            "closure_complete": False,
        },
        "runtime_admissions": refs,
        "coverage": {
            "module_loads": modules,
            "retained_owners": [
                {
                    "kind": "retained-code-source-analysis",
                    "helper": "caller_source_owners.py",
                    "helper_sha256": digest(
                        Path(caller_source_owners.__file__).read_bytes()
                    ),
                },
                *owners,
            ],
            "intern_sinks": [
                {
                    "kind": "unadmitted-native-candidate-analysis",
                    "applied_to_state": False,
                    "sources": sorted(
                        set(sources)
                        - set(FROZEN.values())
                        - set(SOURCE_MODULES.values())
                    ),
                    "candidate_rule": "decoded-adjacent-C-literals-and-word-tokens; generic support and quoted includes",
                }
            ],
            "dynamic_name_recipes": [
                {
                    "kind": "enum-deletion-cache-names",
                    "source": "Lib/enum.py",
                    "strings": sorted(progress),
                    "membership": "possible",
                },
                {
                    "kind": "private-slot-mangling",
                    "source": "Python/symtable.c",
                    "strings": sorted(mangled),
                    "membership": "possible",
                },
                {
                    "kind": "functools-CacheInfo-namedtuple",
                    "source": "Lib/collections/__init__.py",
                    "fields": list(cache_fields),
                    "generated_source": cache_source,
                },
            ],
            "unresolved": [
                {
                    "kind": "absence-closure-unadmitted",
                    "detail": "Only the retained-positive families are proved. Candidate inventory membership provides no absence guarantee; every unlisted spelling remains unknown.",
                }
            ],
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cpython", required=True, type=Path)
    parser.add_argument("--startup-profile", required=True, type=Path)
    parser.add_argument("--runtime-admission", action="append", default=[], type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    options = parser.parse_args()
    result = generate(
        options.cpython,
        options.startup_profile,
        options.runtime_admission,
        options.output,
    )
    serialized = json.dumps(result, indent=2, ensure_ascii=True) + "\n"
    if options.check:
        if options.output.read_text() != serialized:
            raise ValueError("caller source proof differs from generated facts")
    else:
        options.output.write_text(serialized)
    print(
        json.dumps(
            {
                "retained_strings": len(result["state"]["retained_strings"]),
                "possible_ascii_words": len(result["state"]["possible_ascii_words"]),
                "closure_complete": result["state"]["closure_complete"],
            }
        )
    )


if __name__ == "__main__":
    main()
