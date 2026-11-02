#!/usr/bin/env python3
"""Compare serc and pycdc on caches compiled by the running CPython."""

from __future__ import annotations

import argparse
import importlib.util
import json
import marshal
import subprocess
import sys
import sysconfig
import tempfile
import types
import warnings
from collections import Counter, defaultdict, deque
from pathlib import Path

# Isolated mode excludes the script directory from sys.path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_decompile_corpus as corpus


def scopes(code):
    yield code
    for value in code.co_consts:
        if isinstance(value, types.CodeType):
            yield from scopes(value)


def measure(command, cache, filename, original, counts):
    try:
        run = subprocess.run(
            [*command, cache], capture_output=True, check=False, timeout=30
        )
    except subprocess.TimeoutExpired:
        counts["timeouts"] += 1
        return
    if run.returncode:
        counts["nonzero_exits"] += 1
    if not run.stdout.strip():
        return
    try:
        recovered = compile(run.stdout, filename, "exec", dont_inherit=True, optimize=0)
    except (ValueError, SyntaxError, RecursionError):
        counts["invalid_source"] += 1
        return
    counts["compilable_modules"] += 1
    # Pair repeated definitions of the same qualified name in constant-table
    # order. Each comparison includes nested code and the types of constants.
    candidates = defaultdict(deque)
    for scope in scopes(recovered):
        candidates[scope.co_qualname].append(scope)
    for scope in scopes(original):
        peers = candidates.get(scope.co_qualname)
        if peers and corpus.fingerprint(scope) == corpus.fingerprint(peers.popleft()):
            counts["matching_code_objects"] += 1
            counts["matching_modules"] += scope is original


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serc", type=Path)
    parser.add_argument("pycdc", type=Path)
    parser.add_argument("--corpus", choices=("stdlib", "ruff"), default="stdlib")
    args = parser.parse_args()
    sys.path.insert(0, str(corpus.ROOT / "crates/serc/scripts"))
    import generate_cpython_target as target

    minor = f"{sys.version_info.major}.{sys.version_info.minor}"
    version = sys.version.split()[0]
    if (
        sys.implementation.name != "cpython"
        or minor not in target.TARGETS
        or target.TARGETS[minor][0] != f"v{version}"
    ):
        parser.error("run this script with a serc-pinned CPython")
    if args.corpus == "stdlib":
        root = Path(sysconfig.get_path("stdlib"))
        files = sorted(root.glob("*.py"))
    else:
        subprocess.run(
            [corpus.ROOT / "scripts/check-ruff-pin"], check=True, stdout=sys.stderr
        )
        root = corpus.RUFF
        files = sorted(
            path
            for entry in corpus.ROOTS
            for path in (root / "crates" / entry).rglob("*.py")
            if not path.is_symlink()
        )
    counts = Counter()
    totals = {
        tool: Counter(compilable_modules=0, matching_modules=0, matching_code_objects=0)
        for tool in ("serc", "pycdc")
    }
    commands = {
        "serc": [args.serc.resolve(), "decompile"],
        "pycdc": [args.pycdc.resolve()],
    }
    with warnings.catch_warnings(), tempfile.TemporaryDirectory() as scratch:
        warnings.simplefilter("ignore", SyntaxWarning)
        cache = Path(scratch) / "input.pyc"
        for path in files:
            filename = str(path.relative_to(root))
            try:
                code = compile(
                    path.read_bytes(), filename, "exec", dont_inherit=True, optimize=0
                )
            except (SyntaxError, ValueError):
                counts["cpython_rejected"] += 1
                continue
            counts["modules"] += 1
            counts["code_objects"] += corpus.object_count(code)
            cache.write_bytes(
                importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(code)
            )
            for tool, command in commands.items():
                measure(command, cache, filename, code, totals[tool])
    print(
        json.dumps(
            {"python": version, "corpus": args.corpus, "inputs": counts, **totals},
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
