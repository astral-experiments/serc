from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import pgo_corpus as corpus


class PgoCorpusTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write(self, name, contents=b"value = 1\n"):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(contents)
        return path

    def test_copies_only_selected_tracked_production_sources(self):
        project = corpus.Project(
            "example", "owner/example", "1" * 40, ("src/package", "extras")
        )
        selected = ["src/package/module.py", "extras/helper.py"]
        excluded = [
            "src/package/tests/test_module.py",
            "src/package/test/example.py",
            "src/package/_tests/example.py",
            "src/package/vendor/library.py",
            "src/package/_vendor/library.py",
            "src/package/__pycache__/generated.py",
            "src/package/module.pyi",
            "src/package_extra/outside.py",
            "other/outside.py",
        ]
        for name in [*selected, *excluded, "src/package/untracked.py"]:
            self.write(f"checkout/{name}")
        checkout = self.root / "checkout"
        link = checkout / "src/package/linked.py"
        link.symlink_to("module.py")
        before = {
            path: path.read_bytes() for path in checkout.rglob("*") if path.is_file()
        }
        destination = self.root / "private"

        summary = corpus.copy_sources(
            project,
            checkout,
            destination,
            [*selected, *excluded, "src/package/linked.py"],
        )

        self.assertEqual(summary["files"], 2)
        self.assertEqual(
            sorted(
                path.relative_to(destination).as_posix()
                for path in destination.rglob("*.py")
            ),
            sorted(selected),
        )
        self.assertEqual(summary["revision"], project.revision)
        for name in selected:
            self.assertEqual((destination / name).read_bytes(), before[checkout / name])
        (destination / selected[0]).write_bytes(b"private_edit = True\n")
        self.assertEqual(
            {path: path.read_bytes() for path in checkout.rglob("*") if path.is_file()},
            before,
        )
        self.assertTrue(link.is_symlink())

    def test_requires_source_directories_and_nonempty_selection(self):
        checkout = self.root / "checkout"
        destination = self.root / "private"
        project = corpus.Project("example", "owner/example", "1" * 40, ("package",))
        with self.assertRaisesRegex(ValueError, "Missing example source directory"):
            corpus.copy_sources(project, checkout, destination, [])
        self.write("checkout/package/module.pyi")
        with self.assertRaisesRegex(ValueError, "No Python training sources"):
            corpus.copy_sources(project, checkout, destination, ["package/module.pyi"])

    def test_tree_summary_binds_names_and_contents(self):
        self.write("first/a.py", b"alpha")
        self.write("first/nested/b.py", b"beta")
        self.write("second/nested/b.py", b"beta")
        self.write("second/a.py", b"alpha")
        expected = corpus.tree_summary(self.root / "first", ".py")
        self.assertEqual(expected["files"], 2)
        self.assertEqual(expected["bytes"], 9)
        self.assertEqual(corpus.tree_summary(self.root / "second", ".py"), expected)

        (self.root / "second/a.py").rename(self.root / "second/c.py")
        renamed = corpus.tree_summary(self.root / "second", ".py")
        self.assertEqual(renamed["files"], expected["files"])
        self.assertEqual(renamed["bytes"], expected["bytes"])
        self.assertNotEqual(renamed["sha256"], expected["sha256"])
        self.write("second/c.py", b"Alpha")
        self.assertNotEqual(
            corpus.tree_summary(self.root / "second", ".py")["sha256"],
            renamed["sha256"],
        )

    def test_bytecode_verification_rejects_changed_missing_and_extra_output(self):
        bytecode = b"\x2b\x0e\x0d\x0a" + bytes(12) + b"compiled payload"
        first = self.write("corpus/__pycache__/first.cpython-314.pyc", bytecode)
        self.write("corpus/nested/__pycache__/second.cpython-314.pyc", bytecode)
        root = self.root / "corpus"
        expected = corpus.tree_summary(root, ".pyc")
        self.assertEqual(corpus.verify_bytecode(root, expected), expected)

        # A changed header must fail even when file counts and lengths agree.
        first.write_bytes(bytecode[:8] + b"\x01" + bytecode[9:])
        with self.assertRaisesRegex(ValueError, "contents differ from CPython"):
            corpus.verify_bytecode(root, expected)
        first.unlink()
        with self.assertRaisesRegex(ValueError, "contents differ from CPython"):
            corpus.verify_bytecode(root, expected)
        first.write_bytes(bytecode)
        self.write("corpus/extra.pyc", bytecode)
        with self.assertRaisesRegex(ValueError, "contents differ from CPython"):
            corpus.verify_bytecode(root, expected)

    def test_requires_nonempty_profiles_from_requested_group(self):
        directory = self.root / "profiles"
        directory.mkdir()
        with self.assertRaisesRegex(ValueError, "Missing or empty profiles"):
            corpus.require_profiles(directory, "production")
        self.write("profiles/compatibility-1.profraw", b"other group")
        with self.assertRaisesRegex(ValueError, "Missing or empty profiles"):
            corpus.require_profiles(directory, "production")
        profile = self.write("profiles/production-1.profraw", b"")
        with self.assertRaisesRegex(ValueError, "Missing or empty profiles"):
            corpus.require_profiles(directory, "production")
        profile.write_bytes(b"profile data")
        self.write("profiles/compatibility-empty.profraw", b"")
        self.assertEqual(corpus.require_profiles(directory, "production"), [profile])


if __name__ == "__main__":
    unittest.main()
