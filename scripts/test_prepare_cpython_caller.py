from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import prepare_cpython_caller as caller


class CallerRuntimeTests(unittest.TestCase):
    def relocated_entry(self, prefix):
        relative = "lib/python3.14/_sysconfigdata__linux_x86_64-linux-gnu.py"
        path = prefix / relative
        path.parent.mkdir(parents=True)
        content = (
            "# exact generated configuration\n"
            "build_time_vars = {\n"
            f"    'BINDIR': '{prefix}/bin',\n"
            f"    'CFLAGS': '-I{prefix}/include',\n"
            "    'Py_GIL_DISABLED': 0,\n"
            "}\n"
        ).encode()
        path.write_bytes(content)
        item = {
            "path": relative,
            "symlink_target": None,
            "executable": False,
            "prefix_relocation": {
                "kind": "uv-sysconfig-prefix-v1",
                "occurrences": 2,
                "uv_source_commit": caller.UV_RELOCATION_COMMIT,
            },
        }
        item["size"], item["sha256"] = caller.file_identity(prefix, item)
        return item, content

    @unittest.skipUnless(sys.platform == "linux", "qualified Linux runtime")
    def test_prefix_relocation_authenticates_all_other_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            item, content = self.relocated_entry(source)
            path = source / item["path"]
            self.assertEqual(caller.validate_file(source, item), path)
            for changed in (
                content.replace(b"Py_GIL_DISABLED", b"different_key"),
                content.replace(b"': 0", b"': 1"),
                content.replace(b"exact generated", b"changed generated"),
                content.replace(b"    'BINDIR'", b"  'BINDIR'"),
                content.replace(str(source).encode(), b"/wrong-prefix"),
                content.replace(str(source).encode(), b"/wrong-prefix", 1),
                content + str(source).encode(),
                content + caller.PREFIX_MARKER,
            ):
                with self.subTest(content=changed):
                    path.write_bytes(changed)
                    with self.assertRaises(ValueError):
                        caller.validate_file(source, item)
            path.write_bytes(content)
            for field, value in (
                ("kind", "arbitrary-normalization"),
                ("uv_source_commit", "unreviewed"),
                ("occurrences", True),
                ("occurrences", 1),
            ):
                changed = copy.deepcopy(item)
                changed["prefix_relocation"][field] = value
                with self.subTest(field=field), self.assertRaises(ValueError):
                    caller.validate_file(source, changed)
            changed = copy.deepcopy(item)
            changed["path"] = "unrelated.py"
            (source / changed["path"]).write_bytes(content)
            with self.assertRaisesRegex(ValueError, "unsupported runtime prefix"):
                caller.validate_file(source, changed)

    @unittest.skipUnless(sys.platform == "linux", "qualified Linux runtime")
    def test_preparation_rewrites_only_an_authenticated_prefix(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            destination = Path(temporary) / "different-destination-length"
            destination.mkdir()
            item, content = self.relocated_entry(source)
            admission = {
                "schema_version": 1,
                "kind": "standard-caller-source-runtime",
                "version": "3.14.5",
                "executable": item["path"],
                "files": [item],
                "frozen_modules": [],
                "source_modules": [],
                "native_modules": [],
                "environment": {"LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"},
                "loader_library_path": "lib",
            }
            identity = {
                "implementation": "cpython",
                "version": [3, 14, 5, "final", 0],
                "gil_disabled": False,
                "isolated": 1,
                "no_site": 1,
                "executable": str(destination / item["path"]),
                "prefix": str(destination),
                "paths": [str(destination)],
                "modules": {},
            }
            try:
                with patch.object(
                    caller.subprocess,
                    "run",
                    return_value=SimpleNamespace(stdout=json.dumps(identity)),
                ):
                    caller.prepare(admission, source, destination)
                self.assertEqual((source / item["path"]).read_bytes(), content)
                self.assertEqual(
                    (destination / item["path"]).read_bytes(),
                    content.replace(str(source).encode(), str(destination).encode()),
                )
                self.assertEqual(
                    caller.file_identity(source, item),
                    caller.file_identity(destination, item),
                )
            finally:
                for directory, _, _ in os.walk(destination):
                    Path(directory).chmod(0o700)

    @unittest.skipUnless(sys.platform == "linux", "qualified Linux runtime")
    def test_relocation_rejects_escaped_or_symlink_spelled_prefixes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            alias = root / "alias"
            alias.symlink_to(source, target_is_directory=True)
            for prefix in (alias, root / "with spaces", root / "quote'", root / "é"):
                if not prefix.exists():
                    prefix.mkdir()
                with self.subTest(prefix=prefix), self.assertRaises(ValueError):
                    caller.prefix_bytes(prefix)

    def test_relative_paths_preserve_the_admitted_prefix(self):
        self.assertEqual(
            caller.relative_path("lib/python3.14/os.py"), Path("lib/python3.14/os.py")
        )
        for value in (
            "",
            "/lib/python3.14",
            "../outside",
            "lib/../outside",
            "./lib",
            "lib//os.py",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                caller.relative_path(value)

    def test_file_validation_rejects_changed_runtime_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            file = root / "runtime.py"
            file.write_bytes(b"value = 1\n")
            item = {
                "path": "runtime.py",
                "size": file.stat().st_size,
                "sha256": caller.sha256(file),
                "symlink_target": None,
            }
            self.assertEqual(caller.validate_file(root, item), file)
            file.write_bytes(b"value = 2\n")
            with self.assertRaisesRegex(ValueError, "runtime content changed"):
                caller.validate_file(root, item)

    def test_environment_never_inherits_unreviewed_parent_settings(self):
        admission = {
            "environment": {"LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"},
            "loader_library_path": "lib",
        }
        environment = caller.environment(admission, Path("/qualified"))
        self.assertEqual(
            environment,
            admission["environment"] | {"LD_LIBRARY_PATH": "/qualified/lib"},
        )
        admission["environment"]["LD_PRELOAD"] = "unreviewed"
        with self.assertRaisesRegex(ValueError, "unsupported caller environment"):
            caller.environment(admission, Path("/qualified"))


if __name__ == "__main__":
    unittest.main()
