#!/usr/bin/env python3
"""Optimize a native Linux x86-64 wheel and its standalone executable with LLVM BOLT."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def run(*args):
    subprocess.run(args, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "wheel", type=Path, help="unstripped wheel linked with --emit-relocs"
    )
    parser.add_argument("--training-manifest", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--standalone", required=True, type=Path)
    parser.add_argument("--bolt", default="llvm-bolt-18")
    parser.add_argument(
        "--objcopy", required=True, help="llvm-objcopy from the release Rust toolchain"
    )
    args = parser.parse_args()
    training = json.loads(args.training_manifest.read_text())
    corpus = Path(training["corpus"])
    if training["status"] != "complete" or not corpus.is_dir():
        parser.error("the completed PGO training corpus is required")
    executable = shutil.which(args.bolt)
    if executable is None:
        parser.error(f"BOLT executable not found: {args.bolt}")
    # BOLT locates its instrumentation runtime relative to its executable.
    bolt = Path(executable).resolve()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.out_dir) as temp:
        work = Path(temp).resolve()
        run(sys.executable, "-m", "wheel", "unpack", "-d", work, args.wheel)
        (package,) = work.iterdir()
        (binary,) = package.glob("*.data/scripts/serc")
        counters = work / "layout.fdata"
        instrumented = work / "serc-instrumented"
        run(
            bolt,
            binary,
            "--instrument",
            f"--instrumentation-file={counters}",
            "-o",
            instrumented,
        )
        run(
            instrumented,
            "compile",
            "--force",
            "--quiet",
            "--jobs",
            "1",
            "--python-version",
            "3.14",
            "--optimize",
            "0",
            corpus,
        )
        optimized = work / "serc-optimized"
        # Reuse the original text segment and make obsolete function bodies compressible.
        run(
            bolt,
            binary,
            f"--data={counters}",
            "--use-old-text",
            "--align-text=4096",
            "--align-functions=8",
            "--align-functions-max-bytes=7",
            "--trap-old-code",
            "--reorder-blocks=ext-tsp",
            "--reorder-functions=cdsort",
            "--split-functions",
            "--split-all-cold",
            "--split-eh",
            "-o",
            optimized,
        )
        # The release toolchain preserves BOLT's relocated ELF program headers.
        run(args.objcopy, "--strip-all", optimized, binary)
        run(binary, "--version")
        run(sys.executable, "-m", "wheel", "pack", "-d", work, package)
        (packed,) = work.glob("*.whl")
        # Keep the original platform tag order in the artifact name.
        shutil.copy2(binary, args.standalone)
        packed.replace(args.out_dir / args.wheel.name)


if __name__ == "__main__":
    main()
