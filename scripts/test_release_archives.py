from __future__ import annotations

import runpy
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ARCHIVES = runpy.run_path(str(ROOT / "scripts/build-release-archives.py"))


class ReleaseArchiveTests(unittest.TestCase):
    def test_unix_archive_contains_executable_from_wheel(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = "x86_64-unknown-linux-gnu"
            wheel = (
                root
                / "serc-0.1.5-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl"
            )
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr(
                    "serc-0.1.5.data/scripts/serc", b"\x7fELFunix executable"
                )
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-MIT", b"MIT license"
                )
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-APACHE", b"Apache license"
                )
            path = ARCHIVES["create_archive"](wheel, target, "0.1.5", root)
            with tarfile.open(path) as archive:
                members = archive.getmembers()
                self.assertEqual(
                    [member.name for member in members],
                    [
                        f"serc-{target}/serc",
                        f"serc-{target}/LICENSE-MIT",
                        f"serc-{target}/LICENSE-APACHE",
                    ],
                )
                self.assertEqual(
                    [member.mode for member in members], [0o755, 0o644, 0o644]
                )
                self.assertEqual(
                    [archive.extractfile(member).read() for member in members],
                    [
                        b"\x7fELFunix executable",
                        b"MIT license",
                        b"Apache license",
                    ],
                )

    def test_windows_archive_contains_executable_from_wheel(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = "x86_64-pc-windows-msvc"
            wheel = root / "serc-0.1.5-py3-none-win_amd64.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("serc-0.1.5.data/scripts/serc.exe", b"MZ executable")
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-MIT", b"MIT license"
                )
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-APACHE", b"Apache license"
                )
            path = ARCHIVES["create_archive"](wheel, target, "0.1.5", root)
            with zipfile.ZipFile(path) as archive:
                self.assertEqual(
                    archive.namelist(), ["serc.exe", "LICENSE-MIT", "LICENSE-APACHE"]
                )
                self.assertEqual(archive.read("serc.exe"), b"MZ executable")
                self.assertEqual(archive.read("LICENSE-MIT"), b"MIT license")
                self.assertEqual(archive.read("LICENSE-APACHE"), b"Apache license")

    def test_validation_rejects_changed_executable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = "x86_64-pc-windows-msvc"
            wheel = root / "serc-0.1.5-py3-none-win_amd64.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("serc-0.1.5.data/scripts/serc.exe", b"MZoriginal")
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-MIT", b"MIT license"
                )
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-APACHE", b"Apache license"
                )
            path = ARCHIVES["create_archive"](wheel, target, "0.1.5", root)
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("serc.exe", b"MZchanged")
                archive.writestr("LICENSE-MIT", b"MIT license")
                archive.writestr("LICENSE-APACHE", b"Apache license")
            with self.assertRaisesRegex(ValueError, "contents differ"):
                ARCHIVES["verify_archive"](wheel, target, "0.1.5", path)

    def test_validation_rejects_wrong_wheel_platform(self):
        with tempfile.TemporaryDirectory() as directory:
            wheel = Path(directory) / "serc-0.1.5-py3-none-win32.whl"
            wheel.touch()
            with self.assertRaisesRegex(ValueError, "expected .*win_amd64"):
                ARCHIVES["files_from_wheel"](wheel, "x86_64-pc-windows-msvc", "0.1.5")

    def test_validation_rejects_bundled_libraries(self):
        with tempfile.TemporaryDirectory() as directory:
            wheel = Path(directory) / "serc-0.1.5-py3-none-musllinux_1_1_riscv64.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("serc-0.1.5.data/scripts/serc", b"#!python\n")
                archive.writestr("serc.libs/libgcc_s.so.1", b"shared library")
            with self.assertRaisesRegex(ValueError, "cannot bundle libraries"):
                ARCHIVES["files_from_wheel"](
                    wheel, "riscv64gc-unknown-linux-musl", "0.1.5"
                )

    def test_validation_rejects_python_launcher(self):
        with tempfile.TemporaryDirectory() as directory:
            wheel = Path(directory) / "serc-0.1.5-py3-none-macosx_11_0_arm64.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("serc-0.1.5.data/scripts/serc", b"#!python\n")
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-MIT", b"MIT license"
                )
                archive.writestr(
                    "serc-0.1.5.dist-info/licenses/LICENSE-APACHE", b"Apache license"
                )
            with self.assertRaisesRegex(ValueError, "expected a native executable"):
                ARCHIVES["files_from_wheel"](wheel, "aarch64-apple-darwin", "0.1.5")


if __name__ == "__main__":
    unittest.main()
