#!/usr/bin/env python3
"""Compare complete CLI compilations, deleting __pycache__ before every process.

Run only on a disposable project checkout: cache directories are removed recursively.
Cleanup and byte-for-byte validation are outside the timed region. Each timed
process discovers and reads sources, compiles each file, marshals once, and writes PYC files.
Requires Linux taskset; warm OS caches are retained for both executables.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import platform
import random
import resource
import shutil
import statistics
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("baseline", type=pathlib.Path, help="ordinary release executable")
    p.add_argument("optimized", type=pathlib.Path, help="optimized release executable")
    p.add_argument("project", type=pathlib.Path)
    p.add_argument("output", type=pathlib.Path)
    p.add_argument(
        "--baseline-revision", required=True, help="recorded baseline Git commit"
    )
    p.add_argument("--python", type=pathlib.Path, help="optional CPython 3.14.5 oracle")
    p.add_argument("--samples", type=int, default=30)
    p.add_argument("--warmups", type=int, default=3)
    p.add_argument("--jobs", type=int, default=1)
    p.add_argument("--cpus", default="2")
    p.add_argument("--expected-exit-code", type=int, default=0)
    p.add_argument("--expected-output-count", type=int)
    p.add_argument("--oracle-manifest", type=pathlib.Path)
    a = p.parse_args()
    if a.samples < 2 or a.warmups < 0 or a.jobs < 1:
        p.error("require samples >= 2, warmups >= 0, and jobs >= 1")
    if a.python and a.oracle_manifest:
        p.error("choose --python or --oracle-manifest")
    a.output.parent.mkdir(parents=True, exist_ok=True)
    root = a.project.resolve()
    binaries = {"baseline": a.baseline.resolve(), "optimized": a.optimized.resolve()}
    commands = {
        k: [
            "taskset",
            "-c",
            a.cpus,
            str(v),
            "compile",
            "--quiet",
            "--jobs",
            str(a.jobs),
            "--python-version",
            "3.14",
            "--optimize",
            "0",
            "--invalidation-mode",
            "timestamp",
            str(root),
        ]
        for k, v in binaries.items()
    }
    sources = sorted(root.rglob("*.py"))

    def digest(b):
        return hashlib.sha256(b).hexdigest()

    def sources_manifest():
        return {
            str(f.relative_to(root)): {
                "size": f.stat().st_size,
                "mtime_ns": f.stat().st_mtime_ns,
                "sha256": digest(f.read_bytes()),
            }
            for f in sources
        }

    def outputs():
        return {
            str(f.relative_to(root)): digest(f.read_bytes())
            for f in sorted(root.rglob("*.pyc"))
        }

    def cleanup():
        for d in root.rglob("__pycache__"):
            if d.is_dir():
                shutil.rmtree(d)

    def run(name):
        cleanup()
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        start = time.perf_counter()
        completed = subprocess.run(
            commands[name], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
        )
        elapsed = time.perf_counter() - start
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        assert completed.returncode == a.expected_exit_code, completed.stderr.decode()
        assert completed.stderr == expected_stderr, completed.stderr.decode()
        cpu = {
            "user": after.ru_utime - before.ru_utime,
            "system": after.ru_stime - before.ru_stime,
        }
        current = outputs()
        assert current == expected, f"Output mismatch: {name}"
        return elapsed, cpu

    oracle_identity = None
    oracle = None
    if a.python:
        python = str(a.python.resolve())
        oracle_identity = json.loads(
            subprocess.check_output(
                [
                    python,
                    "-I",
                    "-S",
                    "-c",
                    "import importlib.util,json,sys; print(json.dumps(dict(version=list(sys.version_info),magic=importlib.util.MAGIC_NUMBER.hex(),executable=sys.executable)))",
                ],
                text=True,
            )
        )
        assert oracle_identity["version"] == [3, 14, 5, "final", 0], oracle_identity
        assert oracle_identity["magic"] == "2b0e0d0a", oracle_identity
        cleanup()
        reference = subprocess.run(
            [
                python,
                "-I",
                "-S",
                "-m",
                "compileall",
                "-q",
                "-f",
                "-j",
                "1",
                "-o",
                "0",
                "--invalidation-mode",
                "timestamp",
                str(root),
            ],
            capture_output=True,
            text=True,
        )
        assert reference.returncode == a.expected_exit_code, (
            reference.stdout + reference.stderr
        )
        oracle = outputs()
    elif a.oracle_manifest:
        oracle = json.loads(a.oracle_manifest.read_text())
    cleanup()
    preflight = subprocess.run(
        commands["baseline"], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )
    assert preflight.returncode == a.expected_exit_code, preflight.stderr.decode()
    expected_stderr = preflight.stderr
    expected = outputs()
    if oracle is not None:
        assert expected == oracle, "Baseline output differs from the CPython oracle"
    expected_count = (
        a.expected_output_count if a.expected_output_count is not None else len(sources)
    )
    assert len(expected) == expected_count, (len(expected), expected_count)
    manifest = sources_manifest()
    result = {
        "baseline_revision": a.baseline_revision,
        "oracle": oracle_identity,
        "project_revision": subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
        ).strip(),
        "project": str(root),
        "commands": commands,
        "binary_sha256": {k: digest(v.read_bytes()) for k, v in binaries.items()},
        "oracle_manifest_sha256": digest(a.oracle_manifest.read_bytes())
        if a.oracle_manifest
        else None,
        "expected_exit_code": a.expected_exit_code,
        "expected_stderr": expected_stderr.decode(),
        "output_count": len(expected),
        "source_count": len(sources),
        "source_bytes": sum(v["size"] for v in manifest.values()),
        "source_manifest_sha256": digest(json.dumps(manifest, sort_keys=True).encode()),
        "output_manifest_sha256": digest(json.dumps(expected, sort_keys=True).encode()),
        "cache_conditions": "Remove all __pycache__ directories before each fresh process, outside timing; warm OS filesystem caches; no module reuse; full output hashes checked outside every timing.",
        "machine": platform.uname()._asdict(),
        "cpus": a.cpus,
        "jobs": a.jobs,
        "warmups_per_binary": a.warmups,
        "samples": [],
    }
    for i in range(a.warmups):
        for k in ("baseline", "optimized") if i % 2 == 0 else ("optimized", "baseline"):
            print("warmup", i, k, run(k), flush=True)
    for i in range(a.samples):
        row = {
            "pair": i,
            "order": ["baseline", "optimized"]
            if i % 2 == 0
            else ["optimized", "baseline"],
        }
        for k in row["order"]:
            row[k], row[k + "_cpu"] = run(k)
        result["samples"].append(row)
        a.output.write_text(json.dumps(result, indent=2) + "\n")
        print(row, flush=True)
    assert sources_manifest() == manifest, "Sources changed during measurement"
    stats = {}
    for k in binaries:
        values = [r[k] for r in result["samples"]]
        stats[k] = {
            "mean": statistics.mean(values),
            "stdev": statistics.stdev(values),
            "median": statistics.median(values),
            "min": min(values),
            "max": max(values),
        }
    stats["reduction_percent"] = 100 * (
        1 - stats["optimized"]["mean"] / stats["baseline"]["mean"]
    )
    rng = random.Random(0)
    resamples = []
    rows = result["samples"]
    for _ in range(10000):
        sample = rng.choices(rows, k=len(rows))
        resamples.append(
            100
            * (
                1
                - sum(r["optimized"] for r in sample)
                / sum(r["baseline"] for r in sample)
            )
        )
    resamples.sort()
    stats["paired_bootstrap_95_percent_interval"] = [resamples[250], resamples[9750]]
    result["statistics"] = stats
    a.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(stats, indent=2))


if __name__ == "__main__":
    main()
