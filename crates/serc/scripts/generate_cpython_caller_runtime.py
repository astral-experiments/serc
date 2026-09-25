#!/usr/bin/env python3
"""Inventory a pinned source-only runtime; this does not admit its own output."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_cpython_caller_variants as variants
import generate_cpython_target as target


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        result = hashlib.sha256()
        while chunk := stream.read(1024 * 1024):
            result.update(chunk)
    return result.hexdigest()


def generate(
    prefix: Path,
    checkout: Path,
    version: str,
    distribution: str,
    release: str,
    archive_sha256: str,
) -> dict:
    """Bind runtime bytes and exact module sources without observing intern state."""
    runtime_preparer = variants.runtime_preparer
    if any(
        (prefix / relative).is_file()
        for relative in runtime_preparer.RELOCATABLE_SYSCONFIG
    ):
        # Validate the installation spelling before resolving it: a symlink
        # alias must not silently change which prefix bytes are authenticated.
        runtime_preparer.prefix_bytes(prefix)
    prefix = prefix.resolve()
    frozen, ordinary = variants.module_maps(checkout, version)
    executable = f"bin/python{version}"
    entries = []
    selected = [prefix / executable, *(prefix / "lib").rglob("*")]
    for path in sorted(selected):
        relative = path.relative_to(prefix)
        if (
            path.is_dir()
            or path.suffix == ".pyc"
            or {"__pycache__", "site-packages"}.intersection(relative.parts)
        ):
            continue
        if not path.is_file() or not path.resolve().is_relative_to(prefix):
            raise ValueError(f"unsupported runtime entry: {relative}")
        entry = {
            "path": relative.as_posix(),
            "executable": bool(path.stat().st_mode & 0o111),
            "symlink_target": os.readlink(path) if path.is_symlink() else None,
        }
        if entry["path"] in runtime_preparer.RELOCATABLE_SYSCONFIG:
            entry["prefix_relocation"] = {
                "kind": "uv-sysconfig-prefix-v1",
                "occurrences": path.read_bytes().count(
                    runtime_preparer.prefix_bytes(prefix)
                ),
                "uv_source_commit": runtime_preparer.UV_RELOCATION_COMMIT,
            }
        size, sha256 = runtime_preparer.file_identity(prefix, entry)
        entries.append(
            {
                "path": entry["path"],
                "sha256": sha256,
                "size": size,
                "executable": entry["executable"],
                "symlink_target": entry["symlink_target"],
                **(
                    {"prefix_relocation": entry["prefix_relocation"]}
                    if "prefix_relocation" in entry
                    else {}
                ),
            }
        )
    entries.sort(key=lambda entry: entry["path"])
    for source in {*frozen.values(), *ordinary.values()}:
        expected = target.read_committed_file(checkout, source).encode()
        actual = prefix / "lib" / f"python{version}" / source[4:]
        if actual.read_bytes() != expected:
            raise ValueError(f"installed source does not match exact commit: {source}")
    version_string = ".".join(map(str, target.TARGET_VERSION)) + (
        "rc2" if version == "3.15" else ""
    )
    return {
        "schema_version": 1,
        "kind": "standard-caller-source-runtime",
        "implementation": "cpython",
        "version": version_string,
        "platform": "linux",
        "architecture": "x86_64",
        "gil_enabled": True,
        "executable": executable,
        "stdlib": f"lib/python{version}",
        "build": {
            "distribution": distribution,
            "release": release,
            "source_tag": target.TARGET_TAG,
            "source_commit": target.TARGET_COMMIT,
            "archive_sha256": archive_sha256,
        },
        "environment": {"LC_ALL": "C.UTF-8", "LANG": "C.UTF-8", "TZ": "UTC"},
        "loader_library_path": "lib",
        "files": entries,
        "frozen_modules": [
            {"name": name, "source_path": source}
            for name, source in sorted(frozen.items())
        ],
        "source_modules": [
            {"name": name, "source_path": f"lib/python{version}/{source[4:]}"}
            for name, source in sorted(ordinary.items())
        ],
        "native_modules": [{"name": "marshal", "path": None}],
        "preparer_sha256": digest(
            target.CRATE_ROOT.parents[1] / "scripts/prepare_cpython_caller.py"
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target", choices=sorted(variants.ADDITIONAL_SOURCE), required=True
    )
    parser.add_argument("--prefix", type=Path, required=True)
    parser.add_argument("--cpython", type=Path, required=True)
    parser.add_argument("--distribution", required=True)
    parser.add_argument("--release", required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--qualify", type=Path)
    args = parser.parse_args()
    if len(args.archive_sha256) != 64 or any(
        c not in "0123456789abcdef" for c in args.archive_sha256
    ):
        raise ValueError("an independently verified archive SHA-256 is required")
    admission = generate(
        args.prefix,
        args.cpython,
        args.target,
        args.distribution,
        args.release,
        args.archive_sha256,
    )
    args.output.write_text(variants.serialize(admission))
    if args.qualify is not None:
        args.qualify.mkdir(parents=True, exist_ok=True)
        environment = dict(
            admission["environment"], LD_LIBRARY_PATH=str(args.prefix.resolve() / "lib")
        )
        subprocess.run(
            [
                str(args.prefix.resolve() / admission["executable"]),
                "-B",
                "-I",
                "-S",
                str(target.CRATE_ROOT.parents[1] / "scripts/prepare_cpython_caller.py"),
                str(args.output.resolve()),
                str(args.qualify.resolve()),
            ],
            env=environment,
            check=True,
        )
    print(
        json.dumps(
            {
                "version": admission["version"],
                "files": len(admission["files"]),
                "sha256": digest(args.output),
            }
        )
    )


if __name__ == "__main__":
    main()
