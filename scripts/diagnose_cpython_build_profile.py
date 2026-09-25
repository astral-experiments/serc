#!/usr/bin/env python3
"""Report CPython 3.14.5 build identity and folded complex NaN bits.

Run this script with each oracle interpreter being compared. It records native
behavior without importing Serc or deciding which build profile to support.
"""

from __future__ import annotations

import dis
import hashlib
import importlib.util
import json
import marshal
import platform
import struct
import sys
import sysconfig
from pathlib import Path

CASES = {
    "real_nan_imaginary_denominator": "(1e309 - 1e309 + 1j) / (1 + 2j)",
    "real_nan_real_denominator": "(1e309 - 1e309 + 1j) / (2 + 1j)",
    "imaginary_nan_imaginary_denominator": "(1e309j - 1e309j) / (1 + 2j)",
    "imaginary_nan_real_denominator": "(1e309j - 1e309j) / (2 + 1j)",
}


def file_identity(path: Path) -> dict[str, str]:
    path = path.resolve(strict=True)
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def folded_bits(expression: str) -> dict[str, str]:
    code = compile(
        expression, "<cpython-build-profile>", "eval", dont_inherit=True, optimize=0
    )
    instructions = list(dis.get_instructions(code))
    constants = [
        instruction.argval
        for instruction in instructions
        if instruction.opname in ("LOAD_CONST", "RETURN_CONST")
    ]
    if len(constants) != 1 or type(constants[0]) is not complex:
        raise RuntimeError(f"Expected one folded complex constant: {expression}")
    if any(instruction.opname == "BINARY_OP" for instruction in instructions):
        raise RuntimeError(f"Expression was not fully folded: {expression}")
    value = constants[0]
    return {
        "expression": expression,
        "real_bits": struct.pack(">d", value.real).hex(),
        "imaginary_bits": struct.pack(">d", value.imag).hex(),
        "marshal_constant": marshal.dumps(value).hex(),
    }


def main() -> None:
    if sys.implementation.name != "cpython" or sys.version_info != (3, 14, 5, "final", 0):
        raise SystemExit("Run this diagnostic with the exact CPython 3.14.5 oracle.")
    build = {
        name: sysconfig.get_config_var(name)
        for name in ("CC", "CFLAGS", "PY_CFLAGS_NODIST", "CONFIG_ARGS", "SOABI")
    }
    identity = {
        "version": sys.version,
        "compiler": platform.python_compiler(),
        "platform": platform.platform(),
        "magic_number": importlib.util.MAGIC_NUMBER.hex(),
        "executable": file_identity(Path(sys.executable)),
        "sysconfig": build,
    }
    # Record the shared library in the interpreter's prefix when present. This is
    # not a claim about arbitrary loader overrides such as LD_PRELOAD.
    library_name = sysconfig.get_config_var("LDLIBRARY")
    if library_name:
        library = Path(sys.base_prefix) / "lib" / library_name
        if library.is_file():
            identity["prefix_shared_library"] = file_identity(library)
    print(
        json.dumps(
            {
                "identity": identity,
                "cases": {
                    name: folded_bits(expression) for name, expression in CASES.items()
                },
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
