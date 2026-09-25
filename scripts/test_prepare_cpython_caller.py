from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import prepare_cpython_caller as caller


class CallerRuntimeTests(unittest.TestCase):
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
