#!/usr/bin/env python3
"""Derive retained source-codec strings from each pinned CPython release."""

from __future__ import annotations

import argparse
import difflib
import hashlib
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_cpython_bootstrap as bootstrap
import generate_cpython_target as target

MODULES = ("ascii", "latin_1", "cp1251", "cp1252", "iso8859_15", "koi8_r", "euc_jp", "big5")
PROOF_INPUTS = (
    "Objects/codeobject.c",
    "Objects/dictobject.c",
    "Objects/descrobject.c",
    "Objects/typeobject.c",
    "Python/codecs.c",
    "Python/modsupport.c",
    "Modules/cjkcodecs/_codecs_jp.c",
    "Modules/cjkcodecs/_codecs_tw.c",
    "Modules/cjkcodecs/cjkcodecs.h",
    "Modules/cjkcodecs/multibytecodec.c",
    "Modules/cjkcodecs/clinic/multibytecodec.c.h",
)


def clinic_method_definitions(clinic: str) -> dict[str, str]:
    """Read complete clinic macros; static methods do not use interned descriptors."""
    definitions = {}
    lines = iter(clinic.splitlines())
    for line in lines:
        match = re.match(r"#define\s+(_MULTIBYTECODEC_[A-Z_]+_METHODDEF)\b", line)
        if match is None:
            continue
        macro_lines = [line]
        while macro_lines[-1].rstrip().endswith("\\"):
            macro_lines.append(next(lines))
        definition = "\n".join(macro_lines)
        method = re.search(r'\{\s*"([^"]+)"', definition)
        if method is None or match[1] in definitions:
            raise ValueError("unexpected native codec clinic method definition")
        if "METH_STATIC" in definition:
            raise ValueError("static method names require separate interning proof")
        definitions[match[1]] = method[1]
    return definitions


def generate(checkout: Path, version: str) -> tuple[Path, str]:
    target.select_target_version(version)
    target.verify_checkout(checkout)
    expected_release = ("candidate", 2) if version == "3.15" else ("final", 0)
    if (
        sys.implementation.name != "cpython"
        or tuple(sys.version_info[:3]) != target.TARGET_VERSION
        or tuple(sys.version_info[3:]) != expected_release
    ):
        raise ValueError(f"generation requires exact CPython {target.TARGET_TAG}")

    paths = [f"Lib/encodings/{module}.py" for module in MODULES] + list(PROOF_INPUTS)
    sources = {path: target.read_committed_file(checkout, path) for path in paths}
    if "_PyUnicode_InternImmortal(interp" not in sources["Objects/codeobject.c"]:
        raise ValueError("code-object interning implementation changed")
    if version != "3.12" and not all(
        marker in sources["Objects/codeobject.c"]
        for marker in (
            "_PyUnicode_InternMortal(interp, &co->co_name)",
            "_PyUnicode_InternMortal(interp, &co->co_qualname)",
        )
    ):
        raise ValueError("code-name interning implementation changed")
    if "_PyUnicode_InternImmortal" not in sources["Objects/dictobject.c"]:
        raise ValueError("module dictionary key interning implementation changed")
    if (
        "return PyDict_SetItemString(dict, name, value)"
        not in sources["Python/modsupport.c"]
    ):
        raise ValueError("module dictionary insertion implementation changed")
    if (
        "descr->d_name = PyUnicode_InternFromString(name)"
        not in sources["Objects/descrobject.c"]
    ):
        raise ValueError("descriptor name interning implementation changed")
    type_source = sources["Objects/typeobject.c"]
    if not all(
        marker in type_source
        for marker in ("PyDescr_NewMethod(", "PyDescr_NewMember(", "PyDescr_NewGetSet(")
    ):
        raise ValueError("native type descriptor construction changed")
    if "_PyUnicode_InternImmortal(interp, &v)" not in sources["Python/codecs.c"]:
        raise ValueError("codec-registry interning implementation changed")
    header = sources["Modules/cjkcodecs/cjkcodecs.h"]
    add_mapping = (
        "PyModule_AddObject(module, mhname, capsule)"
        if version == "3.12"
        else "PyModule_Add(module, mhname, capsule)"
    )
    if '"__map_"' not in header or add_mapping not in header:
        raise ValueError("CJK mapping exports changed")
    native = sources["Modules/cjkcodecs/multibytecodec.c"]
    if "_multibytecodec.__create_codec" not in native:
        raise ValueError("multibyte codec export changed")
    blocks = re.findall(
        r"static\s+(?:struct\s+)?(PyMethodDef|PyMemberDef|PyGetSetDef)\s+(\w+)\[\]\s*=\s*\{(.*?)\};",
        native,
        re.S,
    )
    expected_blocks = {
        "codecctx_getsets",
        "multibytecodec_methods",
        "mbiencoder_methods",
        "mbidecoder_methods",
        "mbstreamreader_methods",
        "mbstreamreader_members",
        "mbstreamwriter_methods",
        "mbstreamwriter_members",
        "_multibytecodec_methods",
    }
    if {name for _, name, _ in blocks} != expected_blocks:
        raise ValueError("unexpected native codec descriptor tables")
    if any(len(re.findall(r"\b" + name + r"\b", native)) < 2 for _, name, _ in blocks):
        raise ValueError("unreferenced native codec descriptor table")
    if any(
        kind == "PyMethodDef" and re.search(r'\{\s*"', body) for kind, _, body in blocks
    ):
        raise ValueError("unverified inline native method definition")
    clinic = sources["Modules/cjkcodecs/clinic/multibytecodec.c.h"]
    definitions = clinic_method_definitions(clinic)
    macro_names = set(
        re.findall(
            r"_MULTIBYTECODEC_[A-Z_]+_METHODDEF",
            "\n".join(body for _, _, body in blocks),
        )
    )
    if not macro_names or macro_names != definitions.keys():
        raise ValueError("unexpected native codec method definitions")
    native_strings = {definitions[name] for name in macro_names}
    native_strings.update(
        value for _, _, body in blocks for value in re.findall(r'\{\s*"(\w+)"', body)
    )
    mapping_exports = {}
    for codec, module in (("euc_jp", "jp"), ("big5", "tw")):
        cjk = sources[f"Modules/cjkcodecs/_codecs_{module}.c"]
        mapping_list = re.search(
            r"BEGIN_MAPPINGS_LIST\((\d+)\)(.*?)END_MAPPINGS_LIST", cjk, re.S
        )
        if not mapping_list:
            raise ValueError("missing CJK mapping list")
        mappings = re.findall(
            r"MAPPING_(?:DECONLY|ENCONLY|ENCDEC)\((\w+)\)", mapping_list[2]
        )
        if len(mappings) != int(mapping_list[1]) or len(set(mappings)) != len(mappings):
            raise ValueError("unexpected CJK mapping list")
        mapping_exports[codec] = ["__map_" + mapping for mapping in mappings]

    lines = [
        "// @generated by scripts/generate_source_codec_interns.py; do not edit.",
        f"// CPython {target.TARGET_TAG} ({target.TARGET_COMMIT}).",
        "// Code names are immortal; literals and descriptor names have retained owners.",
    ]
    for path in paths:
        digest = hashlib.sha256(sources[path].encode()).hexdigest()
        lines.append(f"// {path}: sha256 {digest}")
    lines.append("")
    for module in MODULES:
        path = f"Lib/encodings/{module}.py"
        info = bootstrap.derive_module(sources[path], path)
        strings = set(info["immortal_identifiers"])
        # From 3.13, code objects intern their names and qualified names even
        # when they contain dots. In 3.12 only identifier constants are known
        # to be interned; names used in co_names are already included above.
        strings.update(
            value
            for value in info["retained_code_strings"]
            if version != "3.12" or re.fullmatch(r"[A-Za-z0-9_]+", value)
        )
        strings.update(
            value
            for value in info["retained_literal_strings"]
            if re.fullmatch(r"[A-Za-z0-9_]+", value)
        )
        if module in mapping_exports:
            strings.update(mapping_exports[module])
            strings.update(native_strings)
        lines.append(f"pub(super) const {module.upper()}: &[&str] = &[")
        lines.extend(
            f"    {target.rust_string_literal(value)}," for value in sorted(strings)
        )
        lines.extend(["];", ""])
    suffix = target.TARGET_TAG.removeprefix("v").replace(".", "_")
    return (
        target.CRATE_ROOT / "src/target" / f"cpython_{suffix}_source_codecs.rs",
        "\n".join(lines),
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-version", choices=target.TARGETS, required=True)
    parser.add_argument("--cpython", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path, result = generate(args.cpython, args.target_version)
    if args.check:
        existing = path.read_text()
        if existing != result:
            print(
                "".join(
                    difflib.unified_diff(
                        existing.splitlines(True),
                        result.splitlines(True),
                        fromfile=str(path),
                        tofile="generated",
                    )
                ),
                end="",
            )
            return 1
        print(f"verified {path}")
    else:
        path.write_text(result)
        print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
