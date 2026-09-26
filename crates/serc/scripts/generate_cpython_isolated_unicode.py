#!/usr/bin/env python3
"""Derive conditional isolated-target Unicode facts from reviewed source proofs.

This is an offline model generator. It does not execute an interpreter, inspect
intern tables, or qualify any installed runtime. The default startup files bind
the existing target model; their observations are not used to derive absence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
METADATA = SCRIPTS / "target_metadata"
OUTPUT = SCRIPTS.parent / "src" / "target" / "isolated_unicode.rs"

# Whole-artifact pins retain the existing independent source-proof admission.
# Reusing a conditional theorem does not qualify the caller represented by it.
TARGETS = {
    "312": {
        "stem": "cpython_3_12_14",
        "tag": "v3.12.14",
        "commit": "2abcf904b8dac8c999d2b3aac76681abb333798a",
        "unicode": "ee6885df2e1e53e0a2a9dd90d0a5781651950ca66149ba264fe5634609a78a8d",
        "startup": "a360df44273c826ac83a1e20cd3f0651d916ec3904120c059e9288965c6b5b3f",
        "dotted": None,
    },
    "313": {
        "stem": "cpython_3_13_15",
        "tag": "v3.13.15",
        "commit": "4061bc4c35f7c26f25264666d4ba083b93d2f6f9",
        "unicode": "129b0ccff8d9d4a587a28e2d1ab1e7e9ea2c295459609998d9c94ed6a643d923",
        "startup": "185f5860fdb6507fa3f8642c7919993fa6813b7d3a96c1cd4d5919a7f877a4ef",
        "dotted": "9f9834e1f17682ca43c3b241c4d8a682d7b4ce464c4f9c002d44d59669603663",
    },
    "314": {
        "stem": "cpython_3_14_5",
        "tag": "v3.14.5",
        "commit": "5607950ef232dad16d75c0cf53101d9649d89115",
        "unicode": "e8919848bc0dfd2a2376144d85fd80a5a868264fdfb2ef3cdbfc8058f9e5cb5a",
        "startup": "1c0a3051c44c5307327db0f331d259e65a7e648da1cb5b30a0b2f8d9ca059f02",
        "dotted": "f4901d30c138cefdd2cda7f078410503ad17d4f045775db49a04758911c5ce29",
    },
    "315": {
        "stem": "cpython_3_15_0rc2",
        "tag": "v3.15.0rc2",
        "commit": "435c9e5a798c99653e3ab64ce29baed0e4f3dfee",
        "unicode": "365864d6105596ca780e42cfebdddbbfceb580819ff0f2affd315fbe95346644",
        "startup": "bec0bd2dedbe3b38ca68e446c67abf0f8d0f4793b551d9c7036acb0517765d63",
        "dotted": "7fc530a09a859ed29d5e0f13168c000788f671ba9c4893c24a90eec82ac67777",
    },
}
DOTTED = re.compile(
    r"[A-Za-z_][A-Za-z_0-9]*(?:\.(?:[A-Za-z_][A-Za-z_0-9]*|<locals>))+\Z"
)
EXCLUDED_PREFIXES = [
    "GraphemeBreakIterator.",
    "Segment.",
    "UCD.",
    "unicodedata.",
]
POSSIBLE_BITS = {"313": 1, "314": 2, "315": 4}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_pinned(name: str, expected: str, provenance: dict[str, str]) -> dict:
    data = (METADATA / name).read_bytes()
    actual = digest(data)
    if actual != expected:
        raise ValueError(f"reviewed model input changed: {name}")
    provenance[f"scripts/target_metadata/{name}"] = actual
    return json.loads(data)


def check_helper(name: str, expected: str, provenance: dict[str, str]) -> None:
    actual = digest((SCRIPTS / name).read_bytes())
    if actual != expected:
        raise ValueError(f"reviewed proof generator changed: {name}")
    provenance[f"scripts/{name}"] = actual


def derive() -> tuple[dict[str, list[str]], dict[str, str]]:
    """Select conditional positives and possible additions, never new absence facts."""
    arrays = {"EXCLUDED_PREFIXES": EXCLUDED_PREFIXES}
    provenance = {}
    for digits, reviewed in TARGETS.items():
        stem = reviewed["stem"]
        proof = read_pinned(
            f"{stem}_unicode_effects.json", reviewed["unicode"], provenance
        )
        startup = read_pinned(f"{stem}_startup.json", reviewed["startup"], provenance)
        if (
            proof["schema_version"] != 1
            or proof["kind"] != "standard-caller-unicode-parsing-source-proof"
            or proof["target"]["tag"] != reviewed["tag"]
            or proof["target"]["commit"] != reviewed["commit"]
            or startup["target_tag"] != reviewed["tag"]
            or startup["target_commit"] != reviewed["commit"]
            or not startup["isolated"]
            or startup["site_enabled"]
            or not startup["gil_enabled"]
            or proof["semantics"] != "partial-parser-retained-positives"
            or proof["coverage"]["other_additions"] != "unknown"
        ):
            raise ValueError(f"unsupported conditional model inputs: {stem}")
        check_helper(
            "generate_cpython_unicode_effects.py",
            proof["generator_sha256"],
            provenance,
        )
        check_helper(
            "generate_cpython_target.py", proof["target_helper_sha256"], provenance
        )
        retained = proof["retained_strings"]
        if retained != sorted(set(retained)) or len(retained) != 17:
            raise ValueError(f"reviewed retained subset changed: {stem}")
        arrays[f"RETAINED{digits}"] = retained
        if reviewed["dotted"] is None:
            if proof["unchanged_domain"] is not None:
                raise ValueError("Python 3.12 has no reviewed preservation domain")
            continue
        dotted = read_pinned(
            f"{stem}_caller_dotted.json", reviewed["dotted"], provenance
        )
        domain = proof["unchanged_domain"]
        if (
            domain["semantics"] != "incoming-absence-preservation-only"
            or domain["domain"] != "ascii-dotted-identifiers-and-locals"
            or domain["initial_dotted_sha256"] != reviewed["dotted"]
            or domain["excluded_prefixes"] != EXCLUDED_PREFIXES
            or dotted["schema_version"] != 1
            or dotted["kind"] != "standard-caller-scoped-absence"
            or dotted["domain"] != domain["domain"]
            or dotted["target"] != proof["target"]
            or dotted["initial_profile_sha256"] != proof["initial_source_proof_sha256"]
            or dotted["runtime_admission_sha256"] != proof["runtime_admission_sha256"]
            or dotted["scope_complete"] is not True
            or not dotted["source_inputs"]
        ):
            raise ValueError(f"reviewed source closure changed: {stem}")
        check_helper(
            "generate_cpython_caller_dotted.py",
            dotted["generator_sha256"],
            provenance,
        )
        check_helper(
            "caller_dotted_metadata.py", dotted["lexical_helper_sha256"], provenance
        )
        possible = dotted["possible_names"]
        if possible != sorted(set(possible)) or any(
            not DOTTED.fullmatch(value) for value in possible
        ):
            raise ValueError(f"possible-name domain is not closed and sorted: {stem}")
        arrays[f"POSSIBLE_D{digits}"] = possible
    return arrays, provenance


def render() -> str:
    arrays, provenance = derive()
    lines = [
        "// Generated by scripts/generate_cpython_isolated_unicode.py. Do not edit.",
        f"// Generator sha256:{digest(Path(__file__).read_bytes())}",
        "// Conditional isolated-target facts, not installed-runtime qualification.",
        "// Retained arrays are partial positives; possible arrays are not absence facts.",
        "// See scripts/target_metadata/isolated_unicode_model.md for the contract.",
    ]
    lines.extend(
        f"// {path}: sha256:{value}" for path, value in sorted(provenance.items())
    )
    possible_masks = {}
    for digits, bit in POSSIBLE_BITS.items():
        for name in arrays[f"POSSIBLE_D{digits}"]:
            possible_masks[name] = possible_masks.get(name, 0) | bit
    possible_rows = sorted(possible_masks.items())
    for digits, bit in POSSIBLE_BITS.items():
        reconstructed = [name for name, mask in possible_rows if mask & bit]
        if reconstructed != arrays[f"POSSIBLE_D{digits}"]:
            raise ValueError(f"target membership changed while encoding {digits}")
        lines.extend(["", f"pub(crate) const POSSIBLE_D{digits}_MASK: u8 = {bit};"])
    lines.extend(
        ["", "#[rustfmt::skip]", "pub(crate) const POSSIBLE_D: &[(&str, u8)] = &["]
    )
    lines.extend(f"    ({json.dumps(name)}, {mask})," for name, mask in possible_rows)
    lines.append("];")
    for name, values in sorted(arrays.items()):
        if name.startswith("POSSIBLE_D"):
            continue
        lines.extend(["", "#[rustfmt::skip]", f"pub(crate) const {name}: &[&str] = &["])
        lines.extend(f"    {json.dumps(value)}," for value in values)
        lines.append("];")
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = render()
    if args.check:
        if args.output.read_text() != expected:
            raise ValueError(f"isolated Unicode model is out of date: {args.output}")
    else:
        args.output.write_text(expected)


if __name__ == "__main__":
    main()
