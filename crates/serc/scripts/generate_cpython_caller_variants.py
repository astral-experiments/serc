#!/usr/bin/env python3
"""Derive retained-only caller profiles for exact CPython 3.12, 3.13 and 3.15.

No captured intern table contributes to these profiles. Every positive comes
from a fixed source operation with an immortal or retained canonical owner.
Version-specific frozen loading matters: 3.12 initializes deep-frozen code at
startup, while 3.13/3.15 read frozen marshal data. Only the latter automatically
intern code metadata. Ordinary source module/class suites are not live owners.
Unlisted strings remain unknown; these artifacts never claim absence.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import os
import re
import stat
import sys
from pathlib import Path
from types import CodeType

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_cpython_target as target

ORACLE_SHA256 = "6812e016e7e871700001ad9f8f832c5998d553bc33c84afb1745dbd664a965fb"
WORD = re.compile(r"[A-Za-z0-9_]*\Z")
BASE_FROZEN = (
    "importlib._bootstrap",
    "importlib._bootstrap_external",
    "zipimport",
    "abc",
    "codecs",
    "io",
    "_collections_abc",
    "genericpath",
    "ntpath",
    "posixpath",
    "os",
    "stat",
)
BASE_SOURCE = (
    "encodings",
    "encodings.aliases",
    "encodings.utf_8",
    "pathlib",
    "operator",
)
REGEX_SOURCE = (
    "fnmatch",
    "functools",
    "collections",
    "keyword",
    "reprlib",
    "types",
    "enum",
    "re",
    "re._compiler",
    "re._parser",
    "re._constants",
    "re._casefix",
    "copyreg",
)
ADDITIONAL_SOURCE = {
    "3.12": (*REGEX_SOURCE, "warnings", "urllib", "urllib.parse", "ipaddress"),
    "3.13": (
        *REGEX_SOURCE,
        "pathlib._abc",
        "pathlib._local",
        "glob",
        "contextlib",
        "warnings",
    ),
    "3.15": ("pathlib._os",),
}
PROOF_INPUTS = (
    "Objects/codeobject.c",
    "Objects/unicodeobject.c",
    "Python/marshal.c",
    "Python/import.c",
    "Python/pylifecycle.c",
    "Python/pythonrun.c",
    "Python/Python-ast.c",
    "Include/internal/pycore_global_strings.h",
    "Programs/_freeze_module.c",
    "Makefile.pre.in",
    "Python/frozen.c",
    "Tools/build/freeze_modules.py",
    "Modules/main.c",
    "Modules/getpath.py",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def verify_generation_runtime(admission: dict) -> str:
    """Bind this compiler process to a qualified immutable runtime, once."""
    prefix = Path(sys.prefix).resolve()
    executable = prefix / admission["executable"]
    if (
        not sys.flags.isolated
        or not sys.flags.no_site
        or Path(sys.executable).resolve() != executable.resolve()
        or os.environ.get("LD_LIBRARY_PATH") != str(prefix / "lib")
        or any(name in os.environ for name in ("LD_PRELOAD", "LD_AUDIT"))
        or Path("/etc/ld.so.preload").exists()
    ):
        raise ValueError(
            "generation requires the qualified isolated runtime invocation"
        )
    paths = {entry["path"] for entry in admission["files"]}
    actual = set()
    for directory, _, names in os.walk(prefix):
        if stat.S_IMODE(Path(directory).stat().st_mode) & 0o222:
            raise ValueError("generation runtime directory is writable")
        actual.update(
            (Path(directory) / name).relative_to(prefix).as_posix() for name in names
        )
    if actual != paths:
        raise ValueError("generation runtime file set differs from admission")
    admitted_library_paths = set()
    executable_hash = None
    for entry in admission["files"]:
        path = prefix / entry["path"]
        if (
            not path.is_file()
            or not path.resolve().is_relative_to(prefix)
            or path.stat().st_size != entry["size"]
            or stat.S_IMODE(path.stat().st_mode) & 0o222
        ):
            raise ValueError("generation runtime file identity changed")
        if entry["symlink_target"] is None:
            if path.is_symlink():
                raise ValueError("unexpected runtime symlink")
        elif not path.is_symlink() or os.readlink(path) != entry["symlink_target"]:
            raise ValueError("generation runtime symlink changed")
        with path.open("rb") as stream:
            fingerprint = hashlib.file_digest(stream, "sha256").hexdigest()
        if fingerprint != entry["sha256"]:
            raise ValueError("generation runtime file content changed")
        if entry["path"] == admission["executable"]:
            executable_hash = fingerprint
        if path.name.startswith("libpython") and ".so" in path.name:
            admitted_library_paths.add(path.resolve())
    # Dynamic CI launchers contain little compiler code themselves. Verify the
    # mapped libpython implementation too; this observes library paths only.
    mapped_libraries = set()
    for line in Path("/proc/self/maps").read_text().splitlines():
        fields = line.split(maxsplit=5)
        if len(fields) == 6 and fields[5].startswith("/"):
            path = Path(fields[5])
            if path.name.startswith("libpython") and ".so" in path.name:
                mapped_libraries.add(path.resolve())
    if mapped_libraries and not mapped_libraries.issubset(admitted_library_paths):
        raise ValueError(
            "compiler libpython was loaded from outside the admitted runtime"
        )
    if executable_hash is None:
        raise ValueError("runtime admission lacks its compiler executable")
    return executable_hash


def code_tree(code: CodeType):
    yield code
    for constant in code.co_consts:
        if isinstance(constant, CodeType):
            yield from code_tree(constant)


def word_constants(value):
    if isinstance(value, str) and WORD.fullmatch(value):
        yield value
    elif isinstance(value, (tuple, frozenset)):
        for item in value:
            yield from word_constants(item)


def identifiers(code: CodeType) -> set[str]:
    return {
        name
        for child in code_tree(code)
        for name in (
            *child.co_names,
            *child.co_varnames,
            *child.co_cellvars,
            *child.co_freevars,
        )
    }


def claims_module(name: str, version: str) -> bool:
    """Do not claim strings from optional imports that can fail and be cleared."""
    return name != "linecache" and (name != "zipimport" or version == "3.12")


def frozen_positives(code: CodeType, version: str) -> set[str]:
    """Deep-freeze/static or marshal initialization establishes these interns."""
    result = identifiers(code)
    for child in code_tree(code):
        result.update(word_constants(child.co_consts))
        if version != "3.12":
            result.update((child.co_filename, child.co_name, child.co_qualname))
    return result


def module_maps(checkout: Path, version: str) -> tuple[dict[str, str], dict[str, str]]:
    target.select_target_version(version)
    target.verify_checkout(checkout)
    tracked = set(
        target.git_output(
            checkout, "ls-tree", "-r", "--name-only", target.TARGET_COMMIT
        ).splitlines()
    )

    def source_path(name: str) -> str:
        stem = "Lib/" + name.replace(".", "/")
        for path in (stem + ".py", stem + "/__init__.py"):
            if path in tracked:
                return path
        raise ValueError(f"missing exact source for {name}")

    frozen = {name: source_path(name) for name in BASE_FROZEN}
    ordinary = {
        name: source_path(name) for name in (*BASE_SOURCE, *ADDITIONAL_SOURCE[version])
    }
    # Source registration is best-effort under recovered allocation failures.
    # Its module selection is verified, but its exclusive strings stay unknown.
    (frozen if version == "3.15" else ordinary).update(
        {"linecache": source_path("linecache")} if version != "3.12" else {}
    )
    return frozen, ordinary


def generate(
    checkout: Path,
    version: str,
    admission_path: Path,
    proof_path: Path,
    startup_path: Path,
):
    frozen, ordinary = module_maps(checkout, version)
    expected_version = (
        (*target.TARGET_VERSION, "candidate", 2)
        if version == "3.15"
        else (*target.TARGET_VERSION, "final", 0)
    )
    if (
        sys.implementation.name != "cpython"
        or tuple(sys.version_info) != expected_version
    ):
        raise ValueError("generation requires the exact target interpreter")
    if proof_path.parent.resolve() != startup_path.parent.resolve():
        raise ValueError(
            "profile and source provenance must share an artifact directory"
        )
    admission_bytes = admission_path.read_bytes()
    admission = json.loads(admission_bytes)
    compiler_hash = verify_generation_runtime(admission)
    executable_entry = next(
        row for row in admission["files"] if row["path"] == admission["executable"]
    )
    if executable_entry["sha256"] != compiler_hash:
        raise ValueError("generation compiler differs from the admitted executable")
    if admission["build"]["source_commit"] != target.TARGET_COMMIT:
        raise ValueError("runtime admission source revision differs")
    runtime_refs = [
        {
            "path": str(
                admission_path.resolve().relative_to(proof_path.parent.resolve())
            ),
            "sha256": digest(admission_bytes),
        }
    ]
    sources = {
        path: target.read_committed_file(checkout, path) for path in PROOF_INPUTS
    }
    if "_PyUnicode_InternImmortal(interp" not in sources["Objects/codeobject.c"]:
        raise ValueError("identifier interning implementation requires a new audit")
    retained = set(
        target.parse_preinterned_strings(
            sources["Include/internal/pycore_global_strings.h"]
        )
    )
    retained.update(
        target.parse_compiler_interned_strings(sources["Python/Python-ast.c"])
    )
    retained.update(chr(point) for point in range(256))
    retained.add("")
    recipes = [
        {
            "kind": "static-and-interpreter-AST-owners",
            "source_anchors": [
                "Include/internal/pycore_global_strings.h",
                "Python/Python-ast.c:init_identifiers",
                "Objects/unicodeobject.c:intern_static",
            ],
            "modules": [],
        }
    ]
    modules = []
    for name, path in {**frozen, **ordinary}.items():
        source = sources.setdefault(path, target.read_committed_file(checkout, path))
        installed = Path(sys.prefix) / "lib" / f"python{version}" / path[4:]
        if digest(installed.read_bytes()) != digest(source.encode()):
            raise ValueError(f"installed source differs from exact tag: {path}")
        selected = "frozen" if name in frozen else "source"
        modules.append(
            {
                "name": name,
                "source_path": path,
                "selection": selected,
                "positives_claimed": claims_module(name, version),
            }
        )
        if not claims_module(name, version):
            continue
        filename = f"<frozen {name}>" if name in frozen else f"<stdlib>/{path[4:]}"
        code = compile(source, filename, "exec", dont_inherit=True, optimize=0)
        retained.update(identifiers(code))
        if name in frozen:
            retained.update(frozen_positives(code, version))
        if name == "encodings.aliases":
            assignments = [
                node
                for node in ast.parse(source).body
                if isinstance(node, ast.Assign)
                and any(
                    isinstance(binding, ast.Name) and binding.id == "aliases"
                    for binding in node.targets
                )
            ]
            if len(assignments) != 1:
                raise ValueError(
                    "codec alias dictionary requires a new ownership audit"
                )
            aliases = ast.literal_eval(assignments[0].value)
            if not isinstance(aliases, dict) or not all(
                isinstance(key, str) and isinstance(value, str)
                for key, value in aliases.items()
            ):
                raise ValueError("codec aliases are not a literal string dictionary")
            retained.update(
                value
                for value in (*aliases.keys(), *aliases.values())
                if WORD.fullmatch(value)
            )
    recipes.extend(
        [
            {
                "kind": "immortal-code-identifiers",
                "source_anchors": ["Objects/codeobject.c:intern_strings"],
                "modules": [row["name"] for row in modules if row["positives_claimed"]],
            },
            {
                "kind": "deep-frozen-static-interns"
                if version == "3.12"
                else "frozen-marshal-interns",
                "source_anchors": [
                    "Objects/codeobject.c:_PyStaticCode_Init",
                    "Objects/unicodeobject.c:intern_static",
                ]
                if version == "3.12"
                else ["Programs/_freeze_module.c", "Python/marshal.c:r_object"],
                "modules": [name for name in frozen if claims_module(name, version)],
            },
            {
                "kind": "retained-literal-dictionary",
                "source_anchors": ["Lib/encodings/aliases.py:aliases"],
                "modules": ["encodings.aliases"],
            },
        ]
    )
    if version == "3.12":
        sources["Tools/build/deepfreeze.py"] = target.read_committed_file(
            checkout, "Tools/build/deepfreeze.py"
        )
        if "_Py_Deepfreeze_Init" not in sources["Python/pylifecycle.c"]:
            raise ValueError("deep-frozen startup initialization changed")
    getpath_code = compile(
        sources["Modules/getpath.py"],
        "<frozen getpath>",
        "exec",
        dont_inherit=True,
        optimize=0,
    )
    retained.update(frozen_positives(getpath_code, version))
    recipes.append(
        {
            "kind": "marshalled-getpath",
            "source_anchors": ["Modules/getpath.py", "Python/marshal.c:r_object"],
            "modules": [],
        }
    )
    comparator = (target.CRATE_ROOT / "examples/compare_cpython_corpus.rs").read_text()
    oracle = re.search(r'const ORACLE: &str = r#"(.*?)"#;', comparator, re.S)
    if oracle is None or digest(oracle[1].encode()) != ORACLE_SHA256:
        raise ValueError("fixed oracle source differs")
    wrapper = compile(oracle[1], "<string>", "exec", dont_inherit=True, optimize=0)
    retained.update(identifiers(wrapper))
    retained.update(word_constants(wrapper.co_consts))
    if version != "3.12":
        retained.update((wrapper.co_filename, wrapper.co_name, wrapper.co_qualname))
    recipes.append(
        {
            "kind": "suspended-fixed-oracle-frame",
            "source_anchors": ["Objects/codeobject.c"],
            "modules": [],
            "script_sha256": ORACLE_SHA256,
        }
    )
    magic_path = (
        "Lib/importlib/_bootstrap_external.py"
        if version != "3.15"
        else "Include/internal/pycore_magic_number.h"
    )
    sources[magic_path] = target.read_committed_file(checkout, magic_path)
    magic_match = (
        re.search(r"MAGIC_NUMBER\s*=\s*\((\d+)\)", sources[magic_path])
        if version != "3.15"
        else re.search(r"#define\s+PYC_MAGIC_NUMBER\s+(\d+)", sources[magic_path])
    )
    if magic_match is None:
        raise ValueError("target magic declaration changed")
    target_identity = {
        "implementation": "cpython",
        "version": ".".join(map(str, target.TARGET_VERSION)),
        "tag": target.TARGET_TAG,
        "commit": target.TARGET_COMMIT,
        "magic_hex": (int(magic_match[1]).to_bytes(2, "little") + b"\r\n").hex(),
    }
    provenance = {
        "schema_version": 1,
        "kind": "source-derived-caller-initial-state",
        "semantics": "guaranteed-retained-partial",
        "target": target_identity,
        "scope": "isolated-fixed-caller-before-compilation",
        "generator_sha256": digest(Path(__file__).read_bytes()),
        "compiler_executable_sha256": compiler_hash,
        "input_sha256": {
            path: digest(value.encode()) for path, value in sorted(sources.items())
        },
        "runtime_admissions": runtime_refs,
        "recipes": recipes,
        "retained_strings": sorted(retained),
        "retained_runtime_paths": [],
    }
    startup_bytes = serialize(provenance).encode()
    profile = {
        "schema_version": 1,
        "kind": "standard-caller-source-proof",
        "target": target_identity,
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
        "generator_sha256": provenance["generator_sha256"],
        "input_sha256": provenance["input_sha256"],
        "startup_profile_sha256": digest(startup_bytes),
        "state": {
            "semantics": "guaranteed-retained-partial",
            "positive_proof_complete": True,
            "retained_strings": provenance["retained_strings"],
            "retained_runtime_paths": [],
            "possible_ascii_words": sorted(
                value for value in retained if value and WORD.fullmatch(value)
            ),
            "absence_domain": "nonempty-ascii-alnum-underscore",
            "closure_complete": False,
        },
        "runtime_admissions": runtime_refs,
        "coverage": {
            "module_loads": modules,
            "retained_owners": [
                {
                    "kind": "source-derived-initial-state-artifact",
                    "path": startup_path.name,
                    "sha256": digest(startup_bytes),
                }
            ],
            "intern_sinks": recipes,
            "dynamic_name_recipes": [{"kind": "no-dynamic-name-claims"}],
            "unresolved": [
                {
                    "kind": "absence-unproved",
                    "detail": "Every unlisted string remains unknown.",
                }
            ],
        },
    }
    return profile, provenance


def serialize(value) -> str:
    return json.dumps(value, indent=2, ensure_ascii=True) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=sorted(ADDITIONAL_SOURCE), required=True)
    parser.add_argument("--cpython", type=Path, required=True)
    parser.add_argument("--runtime-admission", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--startup-output", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    profile, provenance = generate(
        args.cpython,
        args.target,
        args.runtime_admission,
        args.output,
        args.startup_output,
    )
    for path, value in ((args.output, profile), (args.startup_output, provenance)):
        text = serialize(value)
        if args.check:
            if path.read_text() != text:
                raise ValueError(f"generated artifact differs: {path.name}")
        else:
            path.write_text(text)
    print(
        json.dumps(
            {
                "target": args.target,
                "retained_strings": len(profile["state"]["retained_strings"]),
                "semantics": profile["state"]["semantics"],
            }
        )
    )


if __name__ == "__main__":
    main()
