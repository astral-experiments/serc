"""Test reference-builder relocation without loading any fixture libraries."""

from __future__ import annotations

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).parent
spec = importlib.util.spec_from_file_location(
    "reference_builder", ROOT / "build-cpython-reference.py"
)
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class LibrarySelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.prefix = self.root / "prefix"
        self.libdir = self.prefix / "lib"
        self.libdir.mkdir(parents=True)
        self.library = self.libdir / "libpython3.14.so"
        self.library.touch()
        self.addCleanup(mock.patch.stopall)
        mock.patch.object(builder.sys, "base_prefix", str(self.prefix)).start()
        self.loaded = mock.patch.object(
            builder, "loaded_libpython", return_value=self.library
        ).start()

    def select(self, directory):
        return builder.reference_library(directory, self.library.name)

    def test_existing_configured_library_is_preserved(self):
        self.assertEqual(self.select(self.libdir), self.library)

    def test_missing_configured_directory_uses_verified_prefix(self):
        self.assertEqual(self.select(self.root / "absent"), self.library)
        self.loaded.assert_called_once_with(required=True)

    def test_static_executable_can_use_existing_configured_library(self):
        self.loaded.return_value = None
        self.assertEqual(self.select(self.libdir), self.library)
        self.loaded.assert_called_once_with(required=False)

    def test_existing_directory_with_missing_library_does_not_fallback(self):
        configured = self.root / "configured"
        configured.mkdir()
        with self.assertRaisesRegex(ValueError, "missing"):
            self.select(configured)

    def test_wrong_configured_library_does_not_fallback(self):
        configured = self.root / "configured"
        configured.mkdir()
        (configured / self.library.name).touch()
        with self.assertRaisesRegex(ValueError, "differs"):
            self.select(configured)

    def test_wrong_fallback_library_is_rejected(self):
        other = self.root / "other.so"
        other.touch()
        self.loaded.return_value = other
        with self.assertRaisesRegex(ValueError, "differs"):
            self.select(self.root / "absent")

    def test_missing_fallback_library_is_rejected(self):
        self.library.unlink()
        with self.assertRaises(FileNotFoundError):
            self.select(self.root / "absent")

    def test_external_fallback_symlink_is_rejected(self):
        other = self.root / "other.so"
        other.touch()
        self.library.unlink()
        self.library.symlink_to(other)
        with self.assertRaisesRegex(ValueError, "escapes"):
            self.select(self.root / "absent")

    def test_internal_fallback_symlink_is_admitted(self):
        actual = self.libdir / "libpython3.14.so.1.0"
        self.library.rename(actual)
        self.library.symlink_to(actual.name)
        self.loaded.return_value = actual
        self.assertEqual(self.select(self.root / "absent"), self.library)

    def test_invalid_library_filenames_are_rejected(self):
        for name in ("", "../outside.so", "/outside.so"):
            with (
                self.subTest(name=name),
                self.assertRaisesRegex(ValueError, "filename"),
            ):
                builder.reference_library(self.libdir, name)


class LoadedLibraryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.library = self.root / "libpython3.14.so.1.0"
        self.library.touch()

    @staticmethod
    def mapping(path):
        info = path.stat()
        device = f"{os.major(info.st_dev):02x}:{os.minor(info.st_dev):02x}"
        return f"1000-2000 r-xp 00000000 {device} {info.st_ino} {path}\n"

    def resolve(self, contents, *, required=True):
        with mock.patch.object(Path, "read_text", return_value=contents):
            return builder.loaded_libpython(required=required)

    def test_duplicate_segments_are_not_ambiguous(self):
        self.assertEqual(self.resolve(self.mapping(self.library) * 3), self.library)

    def test_hardlink_aliases_share_one_file_identity(self):
        alias = self.root / "libpython3.14.so"
        os.link(self.library, alias)
        self.assertTrue(
            self.resolve(self.mapping(self.library) + self.mapping(alias)).samefile(
                self.library
            )
        )

    def test_two_distinct_python_libraries_are_rejected(self):
        other = self.root / "libpython3.13.so"
        other.touch()
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.resolve(self.mapping(self.library) + self.mapping(other))

    def test_no_python_library_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.resolve("1000-2000 r-xp 00000000 00:00 0 /other.so\n")

    def test_no_python_library_is_optional_without_relocation(self):
        self.assertIsNone(self.resolve("", required=False))

    def test_deleted_mapping_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "deleted"):
            self.resolve(self.mapping(self.library).rstrip() + " (deleted)\n")

    def test_replaced_file_identity_is_rejected(self):
        mapping = self.mapping(self.library)
        retained = self.root / "old"
        self.library.rename(retained)
        self.library.touch()
        with self.assertRaisesRegex(ValueError, "identity changed"):
            self.resolve(mapping)

    def test_missing_mapped_file_is_rejected(self):
        mapping = self.mapping(self.library)
        self.library.unlink()
        with self.assertRaises(FileNotFoundError):
            self.resolve(mapping)


if __name__ == "__main__":
    unittest.main()
