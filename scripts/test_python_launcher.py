from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sysconfig
import tempfile
import unittest
import venv
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "serc_find_bin", ROOT / "python/serc/_find_serc.py"
)
assert SPEC is not None and SPEC.loader is not None
FINDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FINDER)


class PythonLauncherTests(unittest.TestCase):
    def test_package_location_takes_priority_over_other_installations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            current_scripts = root / "environment" / "bin"
            base_scripts = root / "base" / "bin"
            user_scripts = root / "user" / "bin"
            for scripts in (current_scripts, base_scripts, user_scripts):
                scripts.mkdir(parents=True)
                (scripts / ("serc" + sysconfig.get_config_var("EXE"))).touch()

            for package, expected_scripts, platform in (
                (root / "target" / "serc", root / "target" / "bin", "linux"),
                (
                    root / "prefix" / "lib" / "python3.14" / "site-packages" / "serc",
                    root / "prefix" / "bin",
                    "linux",
                ),
                (
                    root / "windows-prefix" / "Lib" / "site-packages" / "serc",
                    root / "windows-prefix" / "Scripts",
                    "win32",
                ),
            ):
                with self.subTest(package=package):
                    expected_scripts.mkdir(parents=True)
                    executable = expected_scripts / (
                        "serc" + sysconfig.get_config_var("EXE")
                    )
                    executable.touch()

                    def get_path(_name, scheme=None, vars=None):
                        if scheme:
                            return str(user_scripts)
                        if vars:
                            return str(base_scripts)
                        return str(current_scripts)

                    with (
                        patch.object(FINDER, "_module_path", return_value=str(package)),
                        patch.object(
                            FINDER.sysconfig, "get_path", side_effect=get_path
                        ),
                        patch.object(FINDER.sys, "platform", platform),
                    ):
                        self.assertEqual(FINDER.find_serc_bin(), str(executable))

    def test_falls_back_to_environment_scripts(self):
        with tempfile.TemporaryDirectory() as directory:
            scripts = Path(directory)
            executable = scripts / ("serc" + sysconfig.get_config_var("EXE"))
            executable.touch()
            with (
                patch.object(FINDER, "_module_path", return_value=None),
                patch.object(FINDER.sysconfig, "get_path", return_value=str(scripts)),
            ):
                self.assertEqual(FINDER.find_serc_bin(), str(executable))

    @unittest.skipIf(os.name == "nt", "requires Unix executable scripts")
    def test_module_launches_target_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            venv.create(root / "venv")
            scripts = root / "venv" / "bin"
            target = root / "target"
            shutil.copytree(ROOT / "python/serc", target / "serc")
            (target / "bin").mkdir()
            for executable, output in (
                (scripts / "serc", "old environment"),
                (target / "bin" / "serc", "target installation"),
            ):
                executable.write_text(f"#!/bin/sh\nprintf '{output}\\n'\n")
                executable.chmod(0o755)
            result = subprocess.run(
                [scripts / "python", "-m", "serc", "--version"],
                cwd=root,
                env={**os.environ, "PYTHONPATH": str(target)},
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertEqual(result.stdout, "target installation\n")


if __name__ == "__main__":
    unittest.main()
