#!/usr/bin/env python3
"""Build the minimal reference against the CPython interpreter running this script."""

from __future__ import annotations

import argparse
import ctypes
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
        if (
            os.major(info.st_dev),
            os.minor(info.st_dev),
        ) != device or info.st_ino != int(fields[4]):
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
    loaded = loaded_libpython(required=not library_dir.exists())
    relocated = not library_dir.exists() or (
        library.is_file() and loaded is not None and not library.samefile(loaded)
    )
    if relocated:
        # The build-time LIBDIR may still exist after the interpreter is relocated.
        prefix = Path(sys.base_prefix).resolve(strict=True)
        library = prefix / "lib" / library_name
        if not library.resolve(strict=True).is_relative_to(prefix):
            raise ValueError("Relocated libpython escapes the interpreter prefix")
    if not library.is_file():
        raise ValueError(f"CPython shared library is missing: {library}")
    # Some distributions statically embed Python in the executable while also
    # providing the configured shared library for embedding consumers.
    if loaded is not None and not library.samefile(loaded):
        raise ValueError("Selected libpython differs from the running interpreter")
    return library


def macos_library() -> Path:
    """Find the loaded framework or shared library, including relocated Python installs."""

    class DlInfo(ctypes.Structure):
        _fields_ = [
            ("filename", ctypes.c_char_p),
            ("base", ctypes.c_void_p),
            ("symbol_name", ctypes.c_char_p),
            ("symbol", ctypes.c_void_p),
        ]

    dladdr = ctypes.CDLL(None).dladdr
    dladdr.argtypes = [ctypes.c_void_p, ctypes.POINTER(DlInfo)]
    dladdr.restype = ctypes.c_int
    info = DlInfo()
    if not dladdr(
        ctypes.cast(ctypes.pythonapi.Py_GetVersion, ctypes.c_void_p), ctypes.byref(info)
    ):
        raise ValueError(
            "Cannot identify the loaded CPython framework or shared library"
        )
    library = Path(os.fsdecode(info.filename)).resolve(strict=True)
    if not library.is_relative_to(Path(sys.base_prefix).resolve(strict=True)):
        raise ValueError("Loaded libpython escapes the interpreter prefix")
    return library


def windows_library() -> Path:
    """Link the import library belonging to the running interpreter's loaded DLL."""
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    get_module_filename = kernel32.GetModuleFileNameW
    get_module_filename.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_uint32]
    get_module_filename.restype = ctypes.c_uint32
    buffer = ctypes.create_unicode_buffer(32768)
    length = get_module_filename(sys.dllhandle, buffer, len(buffer))
    if not length or length == len(buffer):
        raise ValueError("Cannot identify the running CPython DLL")
    runtime = Path(buffer.value).resolve(strict=True)
    prefix = Path(sys.base_prefix).resolve(strict=True)
    name = (
        sysconfig.get_config_var("LIBRARY")
        or f"python{sys.version_info.major}{sys.version_info.minor}.dll"
    )
    if Path(name).name != name or Path(name).suffix != ".dll":
        raise ValueError("Invalid CPython runtime library filename")
    if runtime != (prefix / name).resolve(strict=True):
        raise ValueError("Loaded CPython DLL differs from the interpreter prefix")
    library = (prefix / "libs" / Path(name).with_suffix(".lib")).resolve(strict=True)
    if not library.is_relative_to(prefix):
        raise ValueError("CPython import library escapes the interpreter prefix")
    return library


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if sys.implementation.name != "cpython" or sys.version_info < (3, 12):
        parser.error("use the target CPython interpreter (3.12 or newer)")
    try:
        if sys.platform == "linux" and sysconfig.get_config_var("Py_ENABLE_SHARED"):
            library = reference_library(
                Path(sysconfig.get_config_var("LIBDIR")),
                sysconfig.get_config_var("LDLIBRARY"),
            )
        elif sys.platform == "darwin":
            library = macos_library()
        elif sys.platform == "win32":
            library = windows_library()
        else:
            parser.error(
                "the reference requires a shared CPython library on Linux, macOS, or Windows"
            )
    except (OSError, ValueError) as error:
        parser.error(str(error))
    library_dir = library.parent
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    definitions = [
        f"SERC_PYTHON_HOME={json.dumps(sys.base_prefix, ensure_ascii=False)}",
        f"SERC_PYTHON_MAGIC={int.from_bytes(importlib.util.MAGIC_NUMBER, 'little')}",
    ]
    includes = [sysconfig.get_path("include"), sysconfig.get_path("platinclude")]
    source = str(Path(__file__).with_name("cpython-reference.c"))
    command = [
        *shlex.split(os.environ.get("CC", "cc")),
        "-std=c11",
        "-O2",
        "-Wall",
        "-Wextra",
        "-Werror",
        *(f"-I{directory}" for directory in includes),
        *(f"-D{definition}" for definition in definitions),
        source,
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
    if sys.platform == "win32":
        command = [
            *shlex.split(os.environ.get("CC", "cl")),
            "/nologo",
            "/std:c11",
            "/O2",
            "/W4",
            "/WX",
            "/MD",
            "/D_CRT_SECURE_NO_WARNINGS",
            *(f"/I{directory}" for directory in includes),
            *(f"/D{definition}" for definition in definitions),
            source,
            str(library),
            f"/Fo{output.with_suffix('.obj')}",
            f"/Fe{output}",
        ]
    subprocess.run(command, check=True)
    identity = subprocess.run(
        [str(output), "--identity"],
        check=True,
        capture_output=True,
        text=True,
        env=os.environ
        | {"PATH": sys.base_prefix + os.pathsep + os.environ.get("PATH", "")},
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
