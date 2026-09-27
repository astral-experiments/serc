#!/usr/bin/env python3
"""Generate caller intern tables from admitted captures and pinned CPython sources.

The fixture records observations and ownership evidence from the pinned runtimes.
Source validation authenticates that evidence's inputs; it does not recreate the
captures by compiling or importing the source files on the host interpreter.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path, PurePosixPath

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "serc/scripts"))
import generate_cpython_target as target

CRATE_ROOT = Path(__file__).resolve().parents[1]
FIXTURE = CRATE_ROOT / "scripts/target_metadata/cpython_compileall_profiles.json"
FIXTURE_SHA256 = "9778735d7b8b75f82a0029199fc91019def0362b37f582430465ba2c31d46b1e"
OUTPUT = CRATE_ROOT / "src/compileall_profiles_data.rs"
VERSIONS = ("3.12", "3.13", "3.14", "3.15")
CALLERS = ("compileall", "py_compile")
PROFILE_COUNT = len(VERSIONS) * len(CALLERS) * 3
ALL_PROFILES = (1 << PROFILE_COUNT) - 1


def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    """Reject duplicate JSON keys, including within provenance records."""
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate fixture key: {key}")
        result[key] = value
    return result


def checksum(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def validate_tables(profile: dict) -> None:
    tables = {}
    coverage = 0
    for name in ("strings", "unknown_strings"):
        rows = profile[name]
        if not isinstance(rows, list):
            raise ValueError(f"{name} must be a list")
        values = {}
        for row in rows:
            if not isinstance(row, list) or len(row) != 2:
                raise ValueError(f"invalid {name} entry")
            spelling, mask = row
            if (
                not isinstance(spelling, str)
                or "/" in spelling
                or any(0xD800 <= ord(character) <= 0xDFFF for character in spelling)
            ):
                raise ValueError(f"{name} contains a path or invalid Unicode string")
            if type(mask) is not int or not 0 < mask <= ALL_PROFILES:
                raise ValueError(f"{name} contains an invalid profile mask")
            if spelling in values:
                raise ValueError(f"duplicate {name} spelling: {spelling!r}")
            values[spelling] = mask
            if name == "strings":
                coverage |= mask
        if list(values) != sorted(values):
            raise ValueError(f"{name} entries must be sorted by spelling")
        tables[name] = values
    if coverage != ALL_PROFILES:
        raise ValueError("retained strings do not cover all 24 profiles")
    for spelling, mask in tables["unknown_strings"].items():
        if tables["strings"].get(spelling, 0) & mask:
            raise ValueError(f"retained and uncertain masks overlap: {spelling!r}")


def validate_profiles(profile: dict) -> None:
    records = profile["profiles"]
    if not isinstance(records, list) or len(records) != PROFILE_COUNT:
        raise ValueError("expected 24 profile provenance records")
    for bit, record in enumerate(records):
        version = VERSIONS[bit // 6]
        caller = CALLERS[(bit // 3) % 2]
        optimize = bit % 3
        if (
            type(record["bit"]) is not int
            or record["bit"] != bit
            or record["target"] != version
            or record["caller"] != caller
            or type(record["optimize"]) is not int
            or record["optimize"] != optimize
        ):
            raise ValueError(f"profile {bit} has an incorrect target, caller, or mode")
        captures = record["captures"]
        expected_modes = (optimize,) if optimize < 2 else (2, 3)
        if not isinstance(captures, list) or len(captures) != 2 * len(expected_modes):
            raise ValueError(f"profile {bit} is missing paired native captures")
        seen = set()
        modes = Counter()
        for capture in captures:
            mode, fixture = capture["mode"], capture["fixture"]
            if type(mode) is not int or mode not in expected_modes:
                raise ValueError(f"profile {bit} has an incorrect native mode")
            if (
                not isinstance(fixture, str)
                or re.fullmatch(r"[A-Za-z0-9_.-]+", fixture) is None
                or (mode, fixture) in seen
            ):
                raise ValueError(
                    f"profile {bit} has duplicate or invalid fixture labels"
                )
            seen.add((mode, fixture))
            modes[mode] += 1
            for field in (
                "canonical_sha256",
                "capture_receipt_sha256",
                "owner_report_sha256",
                "layout_sha256",
            ):
                if not checksum(capture[field]):
                    raise ValueError(f"profile {bit} has an invalid {field}")
        if modes != dict.fromkeys(expected_modes, 2):
            raise ValueError(f"profile {bit} lacks two filenames for each native mode")


def read_fixture() -> dict:
    data = FIXTURE.read_bytes()
    if (
        not checksum(FIXTURE_SHA256)
        or hashlib.sha256(data).hexdigest() != FIXTURE_SHA256
    ):
        raise ValueError("caller profile fixture checksum mismatch")
    profile = json.loads(data, object_pairs_hook=unique_object)
    if type(profile["schema"]) is not int or profile["schema"] != 1:
        raise ValueError("unsupported caller profile fixture schema")
    if not isinstance(profile["targets"], dict) or set(profile["targets"]) != set(
        VERSIONS
    ):
        raise ValueError("fixture must cover exactly the four pinned CPython targets")
    validate_profiles(profile)
    validate_tables(profile)
    return profile


def verify_sources(profile: dict, checkouts: dict[str, Path]) -> None:
    """Validate committed source bytes, independently of checkout modifications."""
    for version in VERSIONS:
        record = profile["targets"][version]
        tag, commit, _, _ = target.TARGETS[version]
        if record["tag"] != tag or record["commit"] != commit:
            raise ValueError(
                f"fixture target {version} differs from the compiler target"
            )
        inputs = record["input_sha256"]
        if not isinstance(inputs, dict) or not inputs:
            raise ValueError(f"target {version} has no source pins")
        target.select_target_version(version)
        target.verify_checkout(checkouts[version])
        for relative, expected in inputs.items():
            path = PurePosixPath(relative)
            if (
                not relative
                or path.is_absolute()
                or path.as_posix() != relative
                or any(part in ("", ".", "..") for part in relative.split("/"))
                or "\\" in relative
                or not checksum(expected)
            ):
                raise ValueError(f"target {version} has an invalid source pin")
            source = target.read_committed_file(checkouts[version], relative)
            if hashlib.sha256(source.encode("utf-8")).hexdigest() != expected:
                raise ValueError(f"CPython {tag} source checksum mismatch: {relative}")


def rust_string(value: str) -> str:
    """Encode exact Unicode scalar values without JSON-only escape sequences."""
    special = {
        "\\": "\\\\",
        '"': '\\"',
        "\0": "\\0",
        "\n": "\\n",
        "\r": "\\r",
        "\t": "\\t",
    }
    parts = []
    for character in value:
        if character in special:
            parts.append(special[character])
        elif 0x20 <= ord(character) < 0x7F:
            parts.append(character)
        else:
            parts.append(f"\\u{{{ord(character):x}}}")
    return '"' + "".join(parts) + '"'


def render(profile: dict) -> str:
    lines = [
        "// @generated by scripts/generate_compileall_profiles.py; do not edit.",
        f"// Fixture sha256: {FIXTURE_SHA256}.",
    ]
    for version in VERSIONS:
        tag, commit, _, _ = target.TARGETS[version]
        lines.append(f"// CPython {tag} ({commit}).")
    lines.extend(
        [
            "// Bit index: target (3.12..3.15) * 6 + caller (compileall, py_compile) * 3 + optimization.",
            "// Optimization 2 includes separate -OO and -OOO observations.",
            "",
        ]
    )
    for field, name in (("strings", "RETAINED"), ("unknown_strings", "UNCERTAIN")):
        lines.extend(
            ["#[rustfmt::skip]", f"pub(super) const {name}: &[(&str, u32)] = &["]
        )
        for value, mask in profile[field]:
            lines.append(
                f"    ({rust_string(value)}, 0x{mask >> 16:04x}_{mask & 0xFFFF:04x}),"
            )
        lines.extend(["];", ""])
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for version in VERSIONS:
        parser.add_argument(
            f"--cpython-{version.replace('.', '')}", required=True, type=Path
        )
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    checkouts = {
        version: getattr(args, f"cpython_{version.replace('.', '')}")
        for version in VERSIONS
    }
    try:
        profile = read_fixture()
        verify_sources(profile, checkouts)
        generated = render(profile)
        if args.check:
            existing = OUTPUT.read_text()
            if existing != generated:
                print(
                    "".join(
                        difflib.unified_diff(
                            existing.splitlines(True),
                            generated.splitlines(True),
                            fromfile=str(OUTPUT),
                            tofile="generated",
                        )
                    ),
                    end="",
                )
                return 1
            print(f"verified {OUTPUT}")
        else:
            OUTPUT.write_text(generated)
            print(f"wrote {OUTPUT}")
    except (
        KeyError,
        OSError,
        TypeError,
        ValueError,
        subprocess.CalledProcessError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
