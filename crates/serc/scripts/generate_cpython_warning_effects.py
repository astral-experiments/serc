#!/usr/bin/env python3
"""Derive partial retained warning effects for a qualified standard caller.

Only code identifiers made immortal while loading the exact warning modules are
claimed. Source compilation does not execute those modules or inspect intern
state. Filtering, formatting, codecs, and every unlisted addition remain unknown.
The generated document requires independent admission alongside its initial-state
proof and immutable source-only runtime; it cannot authorize itself.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_cpython_bootstrap as bootstrap
import generate_cpython_target as target

PROOF_SOURCES = (
    "Objects/codeobject.c",
    "Objects/unicodeobject.c",
    "Python/_warnings.c",
    "Python/frozen.c",
    "Tools/build/freeze_modules.py",
    "Lib/importlib/_bootstrap.py",
    "Lib/importlib/_bootstrap_external.py",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def generate(
    checkout: Path,
    initial_path: Path,
    runtime_path: Path,
    version: str,
    verifier_path: Path,
) -> dict:
    """Bind exact source-derived positives to independently admitted caller facts."""
    target.select_target_version(version)
    target.verify_checkout(checkout)
    expected_release = ("candidate", 2) if version == "3.15" else ("final", 0)
    if sys.implementation.name != "cpython" or tuple(sys.version_info) != (
        *target.TARGET_VERSION,
        *expected_release,
    ):
        raise ValueError(
            f"warning effect generation requires exact {target.TARGET_TAG}"
        )
    modules = {"warnings": "Lib/warnings.py"}
    if version in {"3.14", "3.15"}:
        modules["_py_warnings"] = "Lib/_py_warnings.py"
    initial_bytes = initial_path.read_bytes()
    initial = json.loads(initial_bytes)
    runtime_bytes = runtime_path.read_bytes()
    runtime = json.loads(runtime_bytes)
    if initial["kind"] != "standard-caller-source-proof" or initial["target"] != {
        "implementation": "cpython",
        "version": ".".join(map(str, target.TARGET_VERSION)),
        "tag": target.TARGET_TAG,
        "commit": target.TARGET_COMMIT,
        "magic_hex": importlib.util.MAGIC_NUMBER.hex(),
    }:
        raise ValueError("initial source proof has an unsupported target")
    if not any(
        artifact["sha256"] == digest(runtime_bytes)
        for artifact in initial["runtime_admissions"]
    ):
        raise ValueError("runtime admission is not bound to the initial source proof")
    if (
        runtime["kind"] != "standard-caller-source-runtime"
        or runtime["version"] != target.TARGET_TAG.removeprefix("v")
        or runtime["platform"] != "linux"
        or runtime["architecture"] != "x86_64"
        or not runtime["gil_enabled"]
    ):
        raise ValueError("unsupported warning runtime")
    # Reuse the separately reviewed caller-generation qualifier. A shared
    # launcher hash alone does not identify the libpython compiler it loads.
    spec = importlib.util.spec_from_file_location(
        "warning_runtime_verifier", verifier_path
    )
    if spec is None or spec.loader is None:
        raise ValueError("cannot load the repository runtime verifier")
    verifier = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(verifier_path.resolve().parent))
    spec.loader.exec_module(verifier)
    compiler_hash = verifier.verify_generation_runtime(runtime)
    files = {entry["path"]: entry for entry in runtime["files"]}
    initial_modules = []
    if version in {"3.12", "3.13"}:
        if not any(
            entry.get("name") == "warnings"
            and entry.get("source_path") == "Lib/warnings.py"
            and entry.get("selection") == "source"
            and entry.get("positives_claimed")
            for entry in initial["coverage"]["module_loads"]
        ):
            raise ValueError("legacy warning owner is not proved in the initial caller")
        initial_modules.append("warnings")
    sources = {
        path: target.read_committed_file(checkout, path)
        for path in (*PROOF_SOURCES, *modules.values())
    }
    if "_PyUnicode_InternImmortal(interp" not in sources["Objects/codeobject.c"]:
        raise ValueError("code identifier interning implementation changed")
    frozen_sources = ("Python/frozen.c", "Tools/build/freeze_modules.py")
    for path in frozen_sources:
        if digest(sources[path].encode()) != initial["input_sha256"].get(path):
            raise ValueError(
                f"frozen selection source is not bound to the initial proof: {path}"
            )
    config_paths = [path for path in files if path.endswith("/config.c")]
    if len(config_paths) != 1:
        raise ValueError("runtime must bind one builtin initializer table")
    config_path = config_paths[0]
    config_bytes = (Path(sys.prefix) / config_path).read_bytes()
    if digest(config_bytes) != files[config_path]["sha256"]:
        raise ValueError("builtin initializer table differs from runtime admission")
    builtin_names = sorted(
        set(re.findall(r'\{\s*"([^"\n]+)"\s*,', config_bytes.decode()))
    )
    if not builtin_names or any(name in builtin_names for name in modules):
        raise ValueError("warning modules may select a builtin initializer")
    if any(f'"{name}"' in sources["Python/frozen.c"] for name in modules):
        raise ValueError("warning modules may select frozen code")
    module_facts = []
    retained = set()
    for name, path in modules.items():
        source = sources[path]
        source_hash = digest(source.encode())
        runtime_relative = f"{runtime['stdlib']}/{path.removeprefix('Lib/')}"
        if files[runtime_relative]["sha256"] != source_hash:
            raise ValueError(f"admitted runtime source differs: {name}")
        code = compile(
            source, f"<warning-module:{name}>", "exec", dont_inherit=True, optimize=0
        )
        identifiers = set()
        for child in bootstrap.code_tree(code):
            identifiers.update(child.co_names)
            identifiers.update(child.co_varnames)
            identifiers.update(child.co_cellvars)
            identifiers.update(child.co_freevars)
        if name in initial_modules and not identifiers.issubset(
            initial["state"]["retained_strings"]
        ):
            raise ValueError(
                "initial caller does not retain every claimed warning identifier"
            )
        retained.update(identifiers)
        module_facts.append(
            {
                "name": name,
                "source": path,
                "source_sha256": source_hash,
                "runtime_path": runtime_relative,
                "immortal_code_identifiers": sorted(identifiers),
            }
        )
    return {
        "schema_version": 1,
        "kind": "standard-caller-warning-source-proof",
        "target": initial["target"],
        "initial_source_proof_sha256": digest(initial_bytes),
        "runtime_admission_sha256": digest(runtime_bytes),
        "generator_sha256": digest(Path(__file__).read_bytes()),
        "code_tree_helper_sha256": digest(Path(bootstrap.__file__).read_bytes()),
        "runtime_verifier_sha256": digest(verifier_path.read_bytes()),
        "compiler_executable_sha256": compiler_hash,
        "input_sha256": {
            path: digest(text.encode()) for path, text in sorted(sources.items())
        },
        "loader_selection": {
            "frozen_source_sha256": {
                path: digest(sources[path].encode()) for path in frozen_sources
            },
            "builtin_table_runtime_path": config_path,
            "builtin_table_sha256": digest(config_bytes),
            "builtin_names": builtin_names,
            "source_modules": list(modules),
            "initially_loaded_modules": initial_modules,
            "search_path_contract": "the bound initial proof's pristine importer state and qualified runtime search path",
        },
        "warning_categories": ["SyntaxWarning"],
        "semantics": "partial-warning-retained-positives",
        "retained_strings": sorted(retained),
        "modules": module_facts,
        "conditions": {
            "initial_caller_and_runtime_qualified": True,
            "empty_warnoptions": True,
            "bytes_warning": 0,
            "native_default_warning_dispatch": True,
            "unmodified_import_hooks_and_modules": True,
            "source_only_warning_module_selection": True,
        },
        "coverage": {
            "trigger": (
                "The bound initial caller already loaded warnings.py from source and retains all claimed identifiers. In 3.12 an explicit warning with source=NULL does not force an import; availability alone would be insufficient."
                if version in {"3.12", "3.13"}
                else "Each explicit SyntaxWarning uses the native default filter and display path. The qualified loader constructs the exact warning module code trees before their execution; later invocations retain their immortal identifiers."
            ),
            "owner": "Objects/codeobject.c intern_strings makes co_names and localsplus names immortal in every recursively constructed code object.",
            "fallback": "Native display alone is insufficient: Python/_warnings.c clears ImportError and may use its C fallback. The admitted source-only runtime and pristine import contract are required independently.",
            "unknown": "All other additions, including formatting, file-dependent codec loading, and imported module constants remain unknown. This proof supplies no absence rule or complete effect inventory.",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--python-version", choices=sorted(target.TARGETS), default="3.14"
    )
    parser.add_argument("--cpython", required=True, type=Path)
    parser.add_argument("--initial-proof", required=True, type=Path)
    parser.add_argument("--runtime-admission", required=True, type=Path)
    parser.add_argument(
        "--runtime-verifier",
        type=Path,
        default=Path(__file__).with_name("generate_cpython_caller_variants.py"),
    )
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    proof = generate(
        args.cpython,
        args.initial_proof,
        args.runtime_admission,
        args.python_version,
        args.runtime_verifier,
    )
    serialized = json.dumps(proof, indent=2, ensure_ascii=True) + "\n"
    if args.check:
        if args.output.read_text() != serialized:
            raise ValueError("warning source proof differs from generated facts")
    else:
        args.output.write_text(serialized)
    print(
        json.dumps(
            {
                "retained_strings": len(proof["retained_strings"]),
                "semantics": proof["semantics"],
            }
        )
    )


if __name__ == "__main__":
    main()
