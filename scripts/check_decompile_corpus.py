#!/usr/bin/env python3
"""Measure complete-module decompilation against the pinned Ruff corpus and CPython."""

from __future__ import annotations

import argparse
import importlib.util
import json
import marshal
import re
import struct
import subprocess
import sys
import tempfile
import types
import warnings
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUFF = ROOT / "vendor/ruff"
ROOTS = [
    "ty_python_semantic/resources/corpus",
    "ruff_python_parser/resources/valid",
    "ruff_python_parser/resources/inline/ok",
    "ruff_linter/resources/test/fixtures",
    "ruff_python_formatter/resources/test/fixtures",
]
# Baseline complete modules recovered from this Ruff revision.
MINIMUM_VERIFIED = {
    "3.12.14": 1475,
    "3.13.15": 1435,
    "3.14.5": 1432,
    "3.15.0rc2": 1441,
}
FIELDS = (
    "co_name",
    "co_qualname",
    "co_code",
    "co_flags",
    "co_names",
    "co_varnames",
    "co_freevars",
    "co_cellvars",
    "co_argcount",
    "co_posonlyargcount",
    "co_kwonlyargcount",
    "co_stacksize",
    "co_exceptiontable",
)


def fingerprint(value):
    if isinstance(value, types.CodeType):
        data = (
            tuple(getattr(value, key) for key in FIELDS),
            fingerprint(value.co_consts),
        )
    elif isinstance(value, tuple):
        data = tuple(map(fingerprint, value))
    elif isinstance(value, frozenset):
        data = frozenset(map(fingerprint, value))
    elif isinstance(value, float):
        data = struct.pack(">d", value)
    elif isinstance(value, complex):
        data = struct.pack(">dd", value.real, value.imag)
    elif isinstance(value, slice):
        data = tuple(map(fingerprint, (value.start, value.stop, value.step)))
    elif isinstance(value, dict) or type(value).__name__ == "frozendict":
        data = tuple((fingerprint(k), fingerprint(v)) for k, v in value.items())
    else:
        data = value
    return type(value), data


def object_count(code):
    return 1 + sum(
        object_count(c) for c in code.co_consts if isinstance(c, types.CodeType)
    )


def measure(binary, inputs):
    counts = Counter()
    refusals = Counter()
    errors = {}
    with tempfile.TemporaryDirectory() as scratch:
        cache = Path(scratch) / "input.pyc"
        for path in inputs:
            counts["inputs"] += 1
            filename = str(path.relative_to(RUFF))
            try:
                code = compile(
                    path.read_bytes(), filename, "exec", dont_inherit=True, optimize=0
                )
            except (SyntaxError, ValueError):
                counts["cpython_rejected"] += 1
                continue
            counts["modules"] += 1
            objects = object_count(code)
            counts["code_objects"] += objects
            cache.write_bytes(
                importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(code)
            )
            try:
                run = subprocess.run(
                    [binary, "decompile", cache],
                    capture_output=True,
                    timeout=30,
                    check=False,
                )
            except subprocess.TimeoutExpired:
                errors[filename] = "decompiler timed out"
                continue
            if run.returncode == 2 and not run.stdout:
                reason = run.stderr.decode(errors="replace").strip().rsplit(": ", 1)[-1]
                refusals[re.sub(r" at byte \d+", "", reason)] += 1
                counts["refused"] += 1
                continue
            if run.returncode:
                errors[filename] = f"exit {run.returncode}: {run.stderr[-500:]!r}"
                continue
            try:
                recovered = compile(
                    run.stdout, filename, "exec", dont_inherit=True, optimize=0
                )
            except (SyntaxError, ValueError) as exc:
                errors[filename] = f"invalid source: {exc}"
                continue
            if fingerprint(code) != fingerprint(recovered):
                errors[filename] = "different executable fields after recompilation"
                continue
            counts["verified_modules"] += 1
            counts["verified_code_objects"] += objects
    return dict(counts), refusals, errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "serc", type=Path, help="serc executable built with --features decompile"
    )
    args = parser.parse_args()
    subprocess.run([ROOT / "scripts/check-ruff-pin"], stdout=sys.stderr, check=True)
    # Reuse the compiler metadata generator's exact release pins, including release candidates.
    sys.path.insert(0, str(ROOT / "crates/serc/scripts"))
    import generate_cpython_target as target

    version = sys.version.split()[0]
    minor = f"{sys.version_info.major}.{sys.version_info.minor}"
    if (
        sys.implementation.name != "cpython"
        or minor not in target.TARGETS
        or target.TARGETS[minor][0] != f"v{version}"
    ):
        parser.error(f"expected a serc-pinned CPython interpreter, got {sys.version}")
    target.select_target_version(minor)
    metadata = target.OUTPUT.read_text(encoding="utf-8")
    match = re.search(r"const TARGET_MAGIC_NUMBER: \[u8; 4\] = \[([^\]]+)\]", metadata)
    if (
        not match
        or bytes(int(n, 16) for n in match[1].split(",")) != importlib.util.MAGIC_NUMBER
    ):
        parser.error(f"interpreter magic differs from {target.TARGET_TAG}")
    roots = [RUFF / "crates" / name for name in ROOTS]
    if not all(root.is_dir() for root in roots):
        parser.error("missing Ruff corpus")
    inputs = sorted(
        path for root in roots for path in root.rglob("*.py") if not path.is_symlink()
    )
    expected = subprocess.check_output(
        [ROOT / "scripts/ruff-corpus-expectations", minor], text=True
    ).split()
    if len(inputs) != int(expected[expected.index("--expected-files") + 1]):
        parser.error("Ruff corpus file count has changed")
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", SyntaxWarning)
        counts, refusals, errors = measure(args.serc.resolve(), inputs)
    result = {
        "python": version,
        "cpython_commit": target.TARGET_COMMIT,
        "ruff": subprocess.check_output(
            ["git", "-C", RUFF, "rev-parse", "HEAD"], text=True
        ).strip(),
        "counts": counts,
        "refusals": dict(refusals.most_common()),
        "errors": errors,
    }
    print(json.dumps(result, indent=2, ensure_ascii=True))
    if counts.get("verified_modules", 0) < MINIMUM_VERIFIED[version]:
        print(
            f"decompilation coverage fell below {MINIMUM_VERIFIED[version]} modules",
            file=sys.stderr,
        )
        return 1
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
