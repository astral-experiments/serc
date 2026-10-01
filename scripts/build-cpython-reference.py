#!/usr/bin/env python3
"""Build the minimal reference against the CPython interpreter running this script."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shlex
import subprocess
import sys
import sysconfig
from pathlib import Path


def loaded_libpython(*, required: bool) -> Path | None:
    libraries = {}
    for line in Path("/proc/self/maps").read_text().splitlines():
        fields = line.split(maxsplit=5)
        if len(fields) != 6:
            continue
        name = fields[5]
        path = Path(name.removesuffix(" (deleted)"))
        if not re.fullmatch(r"libpython\d+\.\d+[a-z]*\.so(?:\.\d+)*", path.name):
            continue
        if name.endswith(" (deleted)") or "\\" in name:
            raise ValueError("Mapped libpython has a deleted or escaped path")
        info = path.stat()
        device = tuple(int(part, 16) for part in fields[3].split(":"))
        if (os.major(info.st_dev), os.minor(info.st_dev)) != device or info.st_ino != int(fields[4]):
            raise ValueError("Mapped libpython file identity changed")
        libraries[(info.st_dev, info.st_ino)] = path.resolve(strict=True)
    if not libraries and not required:
        return None
    if len(libraries) != 1:
        raise ValueError("Expected exactly one mapped libpython shared library")
    return next(iter(libraries.values()))


def reference_library(library_dir: Path, library_name: str) -> Path:
    if not library_name or Path(library_name).name != library_name:
        raise ValueError("Invalid CPython shared library filename")
    library = library_dir / library_name
    relocated = not library_dir.exists()
    if relocated:
        # Relocated CI archives can retain their original build-time LIBDIR.
        prefix = Path(sys.base_prefix).resolve(strict=True)
        library = prefix / "lib" / library_name
        if not library.resolve(strict=True).is_relative_to(prefix):
            raise ValueError("Relocated libpython escapes the interpreter prefix")
    if not library.is_file():
        raise ValueError(f"CPython shared library is missing: {library}")
    # Some distributions statically embed Python in the executable while also
    # providing the configured shared library for embedding consumers.
    loaded = loaded_libpython(required=relocated)
    if loaded is not None and not library.samefile(loaded):
        raise ValueError("Selected libpython differs from the running interpreter")
    return library


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if sys.implementation.name != "cpython" or sys.version_info < (3, 12):
        parser.error("use the target CPython interpreter (3.12 or newer)")
    if sys.platform != "linux" or not sysconfig.get_config_var("Py_ENABLE_SHARED"):
        parser.error("the reference requires Linux and a shared CPython library")

    try:
        library = reference_library(
            Path(sysconfig.get_config_var("LIBDIR")),
            sysconfig.get_config_var("LDLIBRARY"),
        )
    except (OSError, ValueError) as error:
        parser.error(str(error))
    library_dir = library.parent
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
        "protocol": "minimal-v2",
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
