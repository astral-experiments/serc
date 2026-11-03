#!/usr/bin/env python3
"""Compare executable scopes, including verified functions in invalid pycdc output."""

from __future__ import annotations

import argparse
import importlib.util
import io
import json
import marshal
import subprocess
import sys
import sysconfig
import tempfile
import tokenize
import warnings
from collections import Counter, defaultdict, deque
from pathlib import Path

# Isolated mode excludes the script directory from sys.path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_decompile_corpus as corpus
from decompile_flow import function_matches, scopes
from decompile_fragments import verified_fragments


def measure(
    command, cache, filename, original, counts, *, check_flow=False, salvage=False
):
    try:
        run = subprocess.run(
            [*command, cache], capture_output=True, check=False, timeout=30
        )
    except subprocess.TimeoutExpired:
        counts["timeouts"] += 1
        return set(), {}, set()
    if run.returncode:
        counts["nonzero_exits"] += 1
    if not run.stdout.strip():
        counts["empty_outputs"] += 1
        if run.returncode:
            return set(), {}, set()
    try:
        recovered = compile(run.stdout, filename, "exec", dont_inherit=True, optimize=0)
    except (ValueError, SyntaxError, RecursionError):
        counts["invalid_source"] += 1
        fragments = (
            verified_fragments(run.stdout, filename, original) if salvage else set()
        )
        if fragments:
            counts["invalid_modules_with_verified_fragments"] += 1
            counts["verified_fragment_code_objects"] += len(fragments)
        return set(), {}, fragments
    counts["compilable_modules"] += 1
    if b"# serc: detached function " in run.stdout:
        sections = sum(
            token.type == tokenize.COMMENT
            and token.string.startswith("# serc: detached function ")
            and " (original line " in token.string
            for token in tokenize.tokenize(io.BytesIO(run.stdout).readline)
        )
        if sections:
            counts["detached_sections"] += sections
            counts["modules_with_detached_sections"] += 1
    # Pair repeated definitions of the same qualified name in constant-table
    # order. Each comparison includes nested code and the types of constants.
    candidates = defaultdict(deque)
    for scope in scopes(recovered):
        candidates[scope.co_qualname].append(scope)
    matches = set()
    occurrences = Counter()
    for scope in scopes(original):
        name = scope.co_qualname
        occurrences[name] += 1
        peers = candidates.get(name)
        if peers and corpus.fingerprint(scope) == corpus.fingerprint(peers.popleft()):
            matches.add((name, occurrences[name]))
            counts["matching_code_objects"] += 1
            counts["matching_modules"] += scope is original
    flow = (
        function_matches(original, recovered, run.stdout, filename)
        if check_flow
        else {}
    )
    return matches, flow, set()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serc", type=Path)
    parser.add_argument("pycdc", type=Path)
    parser.add_argument(
        "--corpus", choices=("stdlib", "stdlib-packages", "ruff"), default="stdlib"
    )
    parser.add_argument(
        "--compare-with",
        type=Path,
        metavar="OLDER_SERC",
        help="check for lost code-object matches and changes to fully placed functions",
    )
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
    if args.corpus in {"stdlib", "stdlib-packages"}:
        root = Path(sysconfig.get_path("stdlib"))
        if args.corpus == "stdlib":
            files = sorted(root.glob("*.py"))
        else:
            # The stdlib directory may also contain installed distributions.
            # Use this exact interpreter's package names without importing them.
            files = sorted(
                path
                for name in sys.stdlib_module_names
                if (root / name / "__init__.py").is_file()
                for path in (root / name).rglob("*.py")
                if not path.is_symlink()
            )
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
    previous = [args.compare_with.resolve(), "decompile"] if args.compare_with else None
    regressions = {}
    flow_regressions = {}
    newly_unplaced = {}
    pycdc_only = {}
    pycdc_fragment_only = {}
    gained = 0
    baseline = Counter()
    packages = defaultdict(lambda: defaultdict(Counter))
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
            objects = corpus.object_count(code)
            counts["code_objects"] += objects
            package = None
            if args.corpus == "stdlib-packages":
                package = packages[path.relative_to(root).parts[0]]
                package["inputs"].update(modules=1, code_objects=objects)
            cache.write_bytes(
                importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(code)
            )
            matched = {}
            pycdc_fragments = set()
            for tool, command in commands.items():
                result = Counter()
                compare = previous is not None and tool == "serc"
                matches, flow, fragments = measure(
                    command,
                    cache,
                    filename,
                    code,
                    result,
                    check_flow=compare,
                    salvage=tool == "pycdc",
                )
                if tool == "pycdc":
                    pycdc_fragments = fragments
                matched[tool] = matches
                if compare:
                    old, old_flow, _ = measure(
                        previous, cache, filename, code, baseline, check_flow=True
                    )
                    gained += len(matches - old)
                    if lost := old - matches:
                        regressions[filename] = sorted(lost)
                    for key in sorted(flow.keys() & old_flow.keys()):
                        if lost_fields := old_flow[key] - flow[key]:
                            flow_regressions.setdefault(filename, []).append(
                                (*key, sorted(lost_fields))
                            )
                    if unplaced := old_flow.keys() - flow.keys():
                        newly_unplaced[filename] = sorted(unplaced)
                totals[tool].update(result)
                if package is not None:
                    package[tool].update(result)
            if extra := pycdc_fragments - matched["serc"]:
                pycdc_fragment_only[filename] = sorted(extra)
            if extra := matched["pycdc"] - matched["serc"]:
                pycdc_only[filename] = sorted(extra)
    report = {
        "python": version,
        "corpus": args.corpus,
        "inputs": counts,
        **totals,
        "pycdc_only": pycdc_only,
        "pycdc_fragment_only": pycdc_fragment_only,
    }
    if packages:
        report["packages"] = packages
    if previous:
        report.update(
            baseline=baseline,
            gained_scopes=gained,
            regressions=regressions,
            flow_regressions=flow_regressions,
            newly_unplaced=newly_unplaced,
        )
    print(json.dumps(report, indent=2))
    return bool(
        regressions
        or (
            args.corpus != "ruff"
            and (pycdc_only or pycdc_fragment_only or flow_regressions)
        )
    )


if __name__ == "__main__":
    sys.exit(main())
