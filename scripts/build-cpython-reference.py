#!/usr/bin/env python3
"""Build the minimal reference against the CPython interpreter running this script."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import shlex
import subprocess
import sys
import sysconfig
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if sys.implementation.name != "cpython" or sys.version_info < (3, 12):
        parser.error("use the target CPython interpreter (3.12 or newer)")
    if sys.platform != "linux" or not sysconfig.get_config_var("Py_ENABLE_SHARED"):
        parser.error("the reference requires Linux and a shared CPython library")

    library_dir = Path(sysconfig.get_config_var("LIBDIR"))
    library = library_dir / sysconfig.get_config_var("LDLIBRARY")
    if not library.is_file():
        parser.error(f"CPython shared library is missing: {library}")
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    command = [
        *shlex.split(os.environ.get("CC", "cc")),
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Werror",
        f"-I{sysconfig.get_path('include')}",
        f"-I{sysconfig.get_path('platinclude')}",
        f"-DSERC_PYTHON_HOME={json.dumps(sys.base_prefix, ensure_ascii=False)}",
        f"-DSERC_PYTHON_MAGIC={int.from_bytes(importlib.util.MAGIC_NUMBER, 'little')}",
        str(Path(__file__).with_name("cpython-reference.c")),
        str(library),
        "-Xlinker",
        "-rpath",
        "-Xlinker",
        str(library_dir),
        *shlex.split(sysconfig.get_config_var("LIBS") or ""),
        *shlex.split(sysconfig.get_config_var("SYSLIBS") or ""),
        "-o",
        str(output),
    ]
    subprocess.run(command, check=True)
    identity = subprocess.run(
        [str(output), "--identity"], check=True, capture_output=True, text=True
    )
    expected = {
        "protocol": "minimal-v1",
        "implementation": "cpython",
        "version": ".".join(map(str, sys.version_info[:3])),
        "release_level": sys.version_info.releaselevel,
        "release_serial": sys.version_info.serial,
        "magic": importlib.util.MAGIC_NUMBER.hex(),
    }
    if json.loads(identity.stdout) != expected:
        parser.error(
            f"built reference has unexpected identity: {identity.stdout.strip()}"
        )
    print(output)


if __name__ == "__main__":
    main()
