#!/usr/bin/env python3
"""Prepare the immutable source-only runtime described by a reviewed caller admission."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path, PurePosixPath

PROBE = """
import importlib, json, sys, sysconfig
request = json.loads(sys.stdin.read())
modules = {}
for name in request:
    module = importlib.import_module(name)
    spec = module.__spec__
    modules[name] = {'origin': spec.origin, 'loader': type(spec.loader).__name__}
print(json.dumps({'executable': sys.executable, 'prefix': sys.prefix,
    'implementation': sys.implementation.name, 'version': list(sys.version_info),
    'gil_disabled': bool(sysconfig.get_config_var('Py_GIL_DISABLED')),
    'isolated': sys.flags.isolated, 'no_site': sys.flags.no_site,
    'paths': sys.path, 'modules': modules}))
"""

PREFIX_MARKER = b"/__SERC_RUNTIME_PREFIX__"
UV_RELOCATION_COMMIT = "635500036e1705961315e86447f0fab0a8ddb309"
RELOCATABLE_SYSCONFIG = frozenset(
    f"lib/python{version}/_sysconfigdata__linux_x86_64-linux-gnu.py"
    for version in ("3.14", "3.15")
)


def relative_path(value: str) -> Path:
    """Accept a normalized path that cannot escape an admitted runtime prefix."""
    path = PurePosixPath(value)
    if (
        not value
        or path.is_absolute()
        or str(path) != value
        or any(part in {"", ".", ".."} for part in path.parts)
    ):
        raise ValueError(f"invalid runtime-relative path: {value!r}")
    return Path(*path.parts)


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
        return digest.hexdigest()


def prefix_bytes(prefix: Path) -> bytes:
    """Accept only canonical, unescaped prefixes in the reviewed uv transform."""
    text = str(prefix)
    allowed = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/_-."
    if (
        not prefix.is_absolute()
        or prefix != prefix.resolve(strict=True)
        or any(character not in allowed for character in text)
        or text == "/"
        or PREFIX_MARKER.decode() in text
    ):
        raise ValueError("unsupported sysconfig installation prefix")
    return text.encode("ascii")


def normalized_content(prefix: Path, item: dict, content: bytes) -> bytes:
    """Authenticate every byte modulo the single reviewed installation prefix."""
    relocation = item.get("prefix_relocation")
    if relocation is None:
        return content
    if (
        item["path"] not in RELOCATABLE_SYSCONFIG
        or item["symlink_target"] is not None
        or not isinstance(relocation, dict)
        or set(relocation) != {"kind", "occurrences", "uv_source_commit"}
        or relocation["kind"] != "uv-sysconfig-prefix-v1"
        or relocation["uv_source_commit"] != UV_RELOCATION_COMMIT
        or type(relocation["occurrences"]) is not int
        or relocation["occurrences"] <= 0
    ):
        raise ValueError("unsupported runtime prefix relocation")
    original = prefix_bytes(prefix)
    if PREFIX_MARKER in content or content.count(original) != relocation["occurrences"]:
        raise ValueError("sysconfig prefix occurrences differ from admission")
    return content.replace(original, PREFIX_MARKER)


def file_identity(prefix: Path, item: dict) -> tuple[int, str]:
    """Return exact raw identity, or full normalized identity for tagged sysconfig."""
    path = prefix / relative_path(item["path"])
    if item.get("prefix_relocation") is None:
        return path.stat().st_size, sha256(path)
    content = normalized_content(prefix, item, path.read_bytes())
    return len(content), hashlib.sha256(content).hexdigest()


def validate_file(prefix: Path, item: dict) -> Path:
    """Check the exact bytes and link identity of one manifest entry."""
    path = prefix / relative_path(item["path"])
    target = item["symlink_target"]
    if target is None:
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"runtime file is not regular: {item['path']}")
    elif not path.is_symlink() or os.readlink(path) != target:
        raise ValueError(f"runtime link changed: {item['path']}")
    if not path.resolve().is_relative_to(prefix.resolve()):
        raise ValueError(f"runtime file escapes prefix: {item['path']}")
    if file_identity(prefix, item) != (item["size"], item["sha256"]):
        raise ValueError(f"runtime content changed: {item['path']}")
    return path


def environment(admission: dict, prefix: Path) -> dict[str, str]:
    """Build the entire environment used by the qualified oracle invocation."""
    declared = admission["environment"]
    if declared != {"LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"}:
        raise ValueError("unsupported caller environment")
    result = dict(declared)
    if admission["loader_library_path"] is not None:
        result["LD_LIBRARY_PATH"] = str(
            prefix / relative_path(admission["loader_library_path"])
        )
    return result


def prepare(admission: dict, supplied: Path, destination: Path) -> dict:
    """Copy and qualify one already-admitted build without changing its source prefix."""
    if (
        admission["schema_version"] != 1
        or admission["kind"] != "standard-caller-source-runtime"
    ):
        raise ValueError("unsupported runtime admission")
    if sys.platform != "linux" or os.uname().machine != "x86_64":
        raise ValueError("caller runtime requires Linux x86_64")
    if Path("/etc/ld.so.preload").exists():
        raise ValueError(
            "caller runtime does not support a system preload configuration"
        )
    # uv uses an absolute installation spelling, not symlink canonicalization.
    # Tagged relocation therefore rejects aliases instead of guessing a prefix.
    if any(item.get("prefix_relocation") for item in admission["files"]):
        prefix_bytes(supplied)
    supplied = supplied.resolve()
    destination = destination.resolve()
    if supplied == destination or destination.is_relative_to(supplied):
        raise ValueError("prepared runtime must be separate from the supplied prefix")
    if any(destination.iterdir()):
        raise ValueError("prepared runtime destination must be empty")
    entries = admission["files"]
    paths = [item["path"] for item in entries]
    if paths != sorted(set(paths)) or not paths:
        raise ValueError("runtime manifest paths must be nonempty, sorted and unique")
    for item in entries:
        path = relative_path(item["path"])
        if path.suffix == ".pyc" or {"__pycache__", "site-packages"}.intersection(
            path.parts
        ):
            raise ValueError(
                "source-only runtime cannot contain bytecode or site packages"
            )
        source = validate_file(supplied, item)
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        if item["symlink_target"] is None:
            if item.get("prefix_relocation") is None:
                shutil.copyfile(source, target)
            else:
                # Source authentication above precedes the exact rewrite. The
                # destination then passes the same full normalized-byte check.
                content = source.read_bytes().replace(
                    prefix_bytes(supplied), prefix_bytes(destination)
                )
                target.write_bytes(content)
            target.chmod(0o555 if item["executable"] else 0o444)
    # Resolve links only after all canonical files exist in the copy.
    for item in entries:
        if item["symlink_target"] is not None:
            (destination / relative_path(item["path"])).symlink_to(
                item["symlink_target"]
            )
        validate_file(destination, item)
    for directory, _, _ in os.walk(destination, topdown=False):
        Path(directory).chmod(0o555)
    executable = destination / relative_path(admission["executable"])
    module_names = [
        item["name"]
        for group in ("frozen_modules", "source_modules", "native_modules")
        for item in admission[group]
    ]
    if len(module_names) != len(set(module_names)):
        raise ValueError("module selection contains duplicates")
    invocation_environment = environment(admission, destination)
    process = subprocess.run(
        [str(executable), "-B", "-I", "-S", "-c", PROBE],
        input=json.dumps(module_names),
        text=True,
        capture_output=True,
        env=invocation_environment,
        check=True,
    )
    identity = json.loads(process.stdout)
    expected_version = admission["version"]
    actual_version = ".".join(map(str, identity["version"][:3]))
    if expected_version.endswith("rc2"):
        version_matches = expected_version == actual_version + "rc2" and identity[
            "version"
        ][3:] == ["candidate", 2]
    else:
        version_matches = expected_version == actual_version and identity["version"][
            3:
        ] == ["final", 0]
    if (
        identity["implementation"] != "cpython"
        or not version_matches
        or identity["gil_disabled"]
        or identity["isolated"] != 1
        or identity["no_site"] != 1
        or Path(identity["executable"]) != executable
        or Path(identity["prefix"]) != destination
        or any(not Path(path).is_relative_to(destination) for path in identity["paths"])
    ):
        raise ValueError(
            "prepared interpreter identity or search path differs from admission"
        )
    for item in admission["frozen_modules"]:
        if identity["modules"][item["name"]]["origin"] != "frozen":
            raise ValueError(f"module is not frozen: {item['name']}")
    for item in admission["source_modules"]:
        observed = identity["modules"][item["name"]]
        expected = str(destination / relative_path(item["source_path"]))
        if observed != {"origin": expected, "loader": "SourceFileLoader"}:
            raise ValueError(f"module source selection differs: {item['name']}")
    for item in admission["native_modules"]:
        observed = identity["modules"][item["name"]]
        if item["path"] is None:
            expected = {"origin": "built-in", "loader": "type"}
        else:
            expected = {
                "origin": str(destination / relative_path(item["path"])),
                "loader": "ExtensionFileLoader",
            }
        if observed != expected:
            raise ValueError(f"native module selection differs: {item['name']}")
    observed_files = set()
    for directory, _, files in os.walk(destination):
        if stat.S_IMODE(Path(directory).stat().st_mode) & 0o222:
            raise ValueError("prepared runtime directory is writable")
        observed_files.update(
            (Path(directory) / name).relative_to(destination).as_posix()
            for name in files
        )
    if observed_files != set(paths):
        raise ValueError("prepared runtime contains unlisted files")
    for item in entries:
        validate_file(destination, item)
    return {
        "prefix": str(destination),
        "executable": str(executable),
        "environment": invocation_environment,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("admission", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    admission = json.loads(args.admission.read_text())
    if admission["preparer_sha256"] != sha256(Path(__file__)):
        raise ValueError("runtime admission does not bind this preparer")
    print(json.dumps(prepare(admission, Path(sys.prefix), args.destination)))


if __name__ == "__main__":
    main()
