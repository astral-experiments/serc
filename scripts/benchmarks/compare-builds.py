#!/usr/bin/env python3
"""Compare fresh CLI compile-and-write runs, with complete output verification."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import random
import shutil
import statistics
import subprocess
import time
from pathlib import Path


def fingerprint(root, suffix):
    result = {}
    for path in sorted(root.rglob(f"*{suffix}")):
        if path.is_file() and (suffix == ".pyc" or "__pycache__" not in path.parts):
            data = path.read_bytes()
            result[str(path.relative_to(root))] = [
                len(data),
                hashlib.sha256(data).hexdigest(),
            ]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("optimized", type=Path)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--cpu", type=int)
    parser.add_argument("--baseline-revision")
    parser.add_argument("--optimized-revision")
    args = parser.parse_args()
    if args.samples < 2 or args.warmups < 0 or args.jobs < 1:
        parser.error("require samples >= 2, warmups >= 0, and jobs >= 1")
    corpus = args.corpus.resolve()
    if args.cpu is not None:
        if not hasattr(os, "sched_setaffinity"):
            parser.error("--cpu requires CPU affinity support")
        os.sched_setaffinity(0, {args.cpu})
    binaries = {
        "baseline": args.baseline.resolve(),
        "optimized": args.optimized.resolve(),
    }
    commands = {
        name: [str(binary), "compile", "--quiet", "--jobs", str(args.jobs), str(corpus)]
        for name, binary in binaries.items()
    }
    sources = fingerprint(corpus, ".py")
    if not sources:
        parser.error("corpus contains no Python sources")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    mtimes = {name: (corpus / name).stat().st_mtime_ns for name in sources}
    result = {
        "commands": commands,
        "samples": [],
        "warmups": args.warmups,
        "revisions": {
            "baseline": args.baseline_revision,
            "optimized": args.optimized_revision,
        },
        "source_files": len(sources),
        "source_bytes": sum(v[0] for v in sources.values()),
        "source_manifest_sha256": hashlib.sha256(
            json.dumps(sources, sort_keys=True).encode()
        ).hexdigest(),
        "corpus_revision": subprocess.check_output(
            ["git", "-C", str(corpus), "rev-parse", "HEAD"], text=True
        ).strip(),
        "binary_sha256": {
            name: hashlib.sha256(binary.read_bytes()).hexdigest()
            for name, binary in binaries.items()
        },
        "platform": platform.platform(),
        "timing_units": "seconds",
        "environment": {
            key: value
            for key, value in os.environ.items()
            if key.startswith(("RAYON_", "MIMALLOC_"))
        },
        "affinity": sorted(os.sched_getaffinity(0))
        if hasattr(os, "sched_getaffinity")
        else None,
        "cache_conditions": "Warm source/page cache; read all source files before each run; remove all __pycache__ directories before each run. No compiled-module reuse. Cleanup, warming and output verification are untimed. Writes and process exit are timed; no fsync.",
    }
    expected = None
    for block in range(-args.warmups, args.samples):
        order = (
            ["baseline", "optimized"] if block % 2 == 0 else ["optimized", "baseline"]
        )
        times = {}
        cpu_times = {}
        for name in order:
            for directory in corpus.rglob("__pycache__"):
                if directory.is_dir():
                    shutil.rmtree(directory)
            for path in sources:
                (corpus / path).read_bytes()
            usage = os.times()
            start = time.perf_counter()
            subprocess.run(commands[name], check=True, stdout=subprocess.DEVNULL)
            times[name] = time.perf_counter() - start
            after = os.times()
            cpu_times[name] = (
                after.children_user
                + after.children_system
                - usage.children_user
                - usage.children_system
            )
            outputs = fingerprint(corpus, ".pyc")
            if len(outputs) != len(sources):
                raise RuntimeError(
                    f"Expected {len(sources)} outputs, got {len(outputs)}"
                )
            if expected is None:
                expected = outputs
            elif outputs != expected:
                raise RuntimeError("Full PYC output differs from baseline")
        print(block, order, times, cpu_times, flush=True)
        if block >= 0:
            result["samples"].append(
                {"order": order, **times, "cpu_seconds": cpu_times}
            )
            args.output.write_text(json.dumps(result, indent=2) + "\n")
    if sources != fingerprint(corpus, ".py") or mtimes != {
        name: (corpus / name).stat().st_mtime_ns for name in sources
    }:
        raise RuntimeError("Source contents or mtimes changed during comparison")
    result["pyc_manifest_sha256"] = hashlib.sha256(
        json.dumps(expected, sort_keys=True).encode()
    ).hexdigest()
    result["pyc_files"] = len(expected)
    result["pyc_bytes"] = sum(v[0] for v in expected.values())
    result["summary"] = {}
    for name in binaries:
        values = [sample[name] for sample in result["samples"]]
        result["summary"][name] = {
            "mean": statistics.mean(values),
            "median": statistics.median(values),
            "stdev": statistics.stdev(values),
            "min": min(values),
            "max": max(values),
        }
    samples = result["samples"]

    def gain(rows):
        return 100 * (
            1
            - statistics.mean(row["optimized"] for row in rows)
            / statistics.mean(row["baseline"] for row in rows)
        )

    rng = random.Random(0)
    bootstrap = sorted(gain(rng.choices(samples, k=len(samples))) for _ in range(10000))
    result["improvement_percent"] = gain(samples)
    result["paired_bootstrap_95_percent"] = [bootstrap[250], bootstrap[9750]]
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                k: result[k]
                for k in [
                    "summary",
                    "improvement_percent",
                    "paired_bootstrap_95_percent",
                ]
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
