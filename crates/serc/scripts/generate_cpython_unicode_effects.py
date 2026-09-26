#!/usr/bin/env python3
"""Bind partial native Unicode parsing effects to a qualified caller and runtime.

This generator reads pinned C sources and already admitted runtime files. It does
not execute unicodedata or inspect an intern table. The optional dotted-name
companion preserves incoming absence only outside its possible-name inventory
and the reviewed native namespaces; it cannot establish absence or undo earlier
unknown effects. Other additions remain unknown. Generated artifacts require
independent admission by the comparator.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_cpython_target as target

METHODS = (
    "bidirectional",
    "category",
    "combining",
    "decimal",
    "decomposition",
    "digit",
    "east_asian_width",
    "is_normalized",
    "lookup",
    "mirrored",
    "name",
    "normalize",
    "numeric",
)
RETAINED = sorted((*METHODS, "UCD", "_ucnhash_CAPI", "ucd_3_2_0", "unidata_version"))
SOURCES = (
    "Include/internal/pycore_ucnhash.h",
    "Modules/unicodedata.c",
    "Modules/unicodedata_db.h",
    "Modules/unicodename_db.h",
    "Objects/capsule.c",
    "Objects/typeobject.c",
    "Tools/build/freeze_modules.py",
    "Modules/clinic/unicodedata.c.h",
    "Objects/moduleobject.c",
    "Objects/object.c",
    "Objects/descrobject.c",
    "Objects/dictobject.c",
    "Objects/unicodeobject.c",
    "Python/modsupport.c",
    "Python/import.c",
    "Python/frozen.c",
    "Parser/pegen.c",
    "Parser/string_parser.c",
    "Lib/importlib/_bootstrap.py",
    "Lib/importlib/_bootstrap_external.py",
)
# These complete source inventories have independent initial-domain admission.
# Binding one does not promote its initial facts into post-event absence facts.
REVIEWED_DOTTED = {
    "3.13": "9f9834e1f17682ca43c3b241c4d8a682d7b4ce464c4f9c002d44d59669603663",
    "3.14": "f4901d30c138cefdd2cda7f078410503ad17d4f045775db49a04758911c5ce29",
    "3.15": "7fc530a09a859ed29d5e0f13168c000788f671ba9c4893c24a90eec82ac67777",
}
DOTTED = re.compile(
    r"[A-Za-z_][A-Za-z_0-9]*(?:\.(?:[A-Za-z_][A-Za-z_0-9]*|<locals>))+\Z"
)
EXCLUDED_PREFIXES = (
    "GraphemeBreakIterator.",
    "Segment.",
    "UCD.",
    "unicodedata.",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def unchanged_domain(
    dotted_path: Path | None,
    initial_bytes: bytes,
    runtime_bytes: bytes,
    version: str,
) -> dict | None:
    """Bind a reviewed complement that preserves, but never creates, absence."""
    if dotted_path is None:
        return None
    dotted_bytes = dotted_path.read_bytes()
    dotted_hash = digest(dotted_bytes)
    if dotted_hash != REVIEWED_DOTTED.get(version):
        raise ValueError("dotted companion lacks independent source admission")
    dotted = json.loads(dotted_bytes)
    initial = json.loads(initial_bytes)
    if (
        dotted["schema_version"] != 1
        or dotted["kind"] != "standard-caller-scoped-absence"
        or dotted["domain"] != "ascii-dotted-identifiers-and-locals"
        or dotted["phase"] != "before-target-source-decoding"
        or dotted["scope_complete"] is not True
        or dotted["target"] != initial["target"]
        or dotted["initial_profile_sha256"] != digest(initial_bytes)
        or dotted["runtime_admission_sha256"] != digest(runtime_bytes)
        or not dotted["source_inputs"]
    ):
        raise ValueError("dotted companion source, target or caller binding differs")
    scripts = Path(__file__).resolve().parent
    for field, filename in (
        ("generator_sha256", "generate_cpython_caller_dotted.py"),
        ("lexical_helper_sha256", "caller_dotted_metadata.py"),
    ):
        if dotted[field] != digest((scripts / filename).read_bytes()):
            raise ValueError("dotted companion generator provenance differs")
    possible = dotted["possible_names"]
    if possible != sorted(set(possible)) or any(
        not DOTTED.fullmatch(value) for value in possible
    ):
        raise ValueError("dotted companion names must be sorted within its domain")
    return {
        "domain": "ascii-dotted-identifiers-and-locals",
        "semantics": "incoming-absence-preservation-only",
        "initial_dotted_sha256": dotted_hash,
        "excluded_prefixes": list(EXCLUDED_PREFIXES),
    }


def generate(
    checkout: Path,
    initial_path: Path,
    runtime_path: Path,
    runtime_root: Path,
    version: str,
    dotted_path: Path | None = None,
) -> dict:
    """Derive retained module keys without treating unlisted strings as absent."""
    target.select_target_version(version)
    target.verify_checkout(checkout)
    initial_bytes = initial_path.read_bytes()
    initial = json.loads(initial_bytes)
    runtime_bytes = runtime_path.read_bytes()
    runtime = json.loads(runtime_bytes)
    expected = {
        "implementation": "cpython",
        "version": ".".join(map(str, target.TARGET_VERSION)),
        "tag": target.TARGET_TAG,
        "commit": target.TARGET_COMMIT,
    }
    if initial["kind"] != "standard-caller-source-proof" or any(
        initial["target"][key] != value for key, value in expected.items()
    ):
        raise ValueError("initial proof does not identify the pinned target")
    if not any(
        item["sha256"] == digest(runtime_bytes)
        for item in initial["runtime_admissions"]
    ):
        raise ValueError("runtime is not admitted by the initial source proof")
    if (
        runtime["kind"] != "standard-caller-source-runtime"
        or runtime["version"] != target.TARGET_TAG.removeprefix("v")
        or runtime["platform"] != "linux"
        or runtime["architecture"] != "x86_64"
        or not runtime["gil_enabled"]
    ):
        raise ValueError("unsupported Unicode parsing runtime")
    sources = {
        path: target.read_committed_file(checkout, path).encode() for path in SOURCES
    }
    module = sources["Modules/unicodedata.c"].decode()
    clinic = sources["Modules/clinic/unicodedata.c.h"].decode()
    for name in METHODS:
        macro = f"UNICODEDATA_UCD_{name.upper()}_METHODDEF"
        if macro not in module or not re.search(
            rf'#define {macro}[^\n]*\\\n\s*\{{"{name}",', clinic
        ):
            raise ValueError(f"native retained method key changed: {name}")
    for token in (
        '"unidata_version"',
        '"unicodedata.UCD"',
        '"ucd_3_2_0"',
        '"_ucnhash_CAPI"',
        "{Py_mod_exec, unicodedata_exec}",
    ):
        if token not in module:
            raise ValueError(f"native retained module/type recipe changed: {token}")
    files = {item["path"]: item for item in runtime["files"]}

    def checked_file(path: str) -> bytes:
        entry = files[path]
        relative = Path(path)
        if relative.is_absolute() or any(
            part in {".", ".."} for part in relative.parts
        ):
            raise ValueError("runtime file path must remain inside its admitted root")
        data = (runtime_root / relative).read_bytes()
        if digest(data) != entry["sha256"] or len(data) != entry["size"]:
            raise ValueError(f"runtime file differs from admission: {path}")
        return data

    configs = [
        path
        for path in files
        if path.startswith(f"lib/python{version}/config-")
        and path.endswith("/config.c")
    ]
    if len(configs) != 1:
        raise ValueError("runtime must bind exactly one native builtin table")
    config_path = configs[0]
    config = checked_file(config_path).decode()
    builtin = bool(
        re.search(r'\{\s*"unicodedata"\s*,\s*PyInit_unicodedata\s*\}', config)
    )
    if builtin:
        carrier = f"lib/libpython{version}.so.1.0"
    else:
        extensions = [
            path
            for path in files
            if path.startswith(f"lib/python{version}/lib-dynload/unicodedata.")
            and path.endswith(".so")
        ]
        if len(extensions) != 1:
            raise ValueError("runtime must bind exactly one native Unicode extension")
        carrier = extensions[0]
        if any(
            path.startswith(
                (
                    f"lib/python{version}/unicodedata.",
                    f"lib/python{version}/unicodedata/",
                )
            )
            for path in files
        ):
            raise ValueError(
                "an earlier standard-library candidate shadows the extension"
            )
    checked_file(carrier)
    libpython_path = f"lib/libpython{version}.so.1.0"
    checked_file(libpython_path)
    checked_file(runtime["executable"])
    return {
        "schema_version": 1,
        "kind": "standard-caller-unicode-parsing-source-proof",
        "target": initial["target"],
        "initial_source_proof_sha256": digest(initial_bytes),
        "runtime_admission_sha256": digest(runtime_bytes),
        "generator_sha256": digest(Path(__file__).read_bytes()),
        "target_helper_sha256": digest(Path(target.__file__).read_bytes()),
        "input_sha256": {path: digest(data) for path, data in sorted(sources.items())},
        "semantics": "partial-parser-retained-positives",
        "retained_strings": RETAINED,
        "unchanged_domain": unchanged_domain(
            dotted_path, initial_bytes, runtime_bytes, version
        ),
        "loader_selection": {
            "kind": "builtin" if builtin else "extension",
            "config_path": config_path,
            "config_sha256": files[config_path]["sha256"],
            "carrier_path": carrier,
            "carrier_sha256": files[carrier]["sha256"],
            "libpython_path": libpython_path,
            "libpython_sha256": files[libpython_path]["sha256"],
            "executable_path": runtime["executable"],
            "executable_sha256": files[runtime["executable"]]["sha256"],
        },
        "conditions": {
            "qualified_initial_caller_and_runtime": True,
            "native_normalizer_and_name_capsule": True,
            "unmodified_import_hooks_and_modules": True,
            "pristine_source_only_runtime": True,
            "standard_source_codecs": True,
            "retained_owners_preserved": True,
        },
        "coverage": {
            "method_keys": list(METHODS),
            "module_keys": ["UCD", "_ucnhash_CAPI", "ucd_3_2_0", "unidata_version"],
            "owners": "native module dictionary and UCD method/member descriptors",
            "other_additions": "unknown",
            "temporary_normalization_results": "not claimed",
            "lookup_caches": "parser-local normalizer and interpreter-wide name capsule",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path)
    parser.add_argument("--initial-source-proof", type=Path, required=True)
    parser.add_argument("--initial-dotted-proof", type=Path)
    parser.add_argument("--runtime-admission", type=Path, required=True)
    parser.add_argument("--runtime-root", type=Path, required=True)
    parser.add_argument("--python-version", choices=target.TARGETS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    document = generate(
        args.checkout,
        args.initial_source_proof,
        args.runtime_admission,
        args.runtime_root,
        args.python_version,
        args.initial_dotted_proof,
    )
    contents = json.dumps(document, indent=2) + "\n"
    if args.check:
        if args.output.read_text() != contents:
            raise SystemExit("Unicode parsing effect artifact is out of date")
    else:
        args.output.write_text(contents)


if __name__ == "__main__":
    main()
