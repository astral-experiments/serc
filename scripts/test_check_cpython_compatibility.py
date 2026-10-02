"""Check native reference routing without compiling the compatibility corpus."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
VERSIONS = ("3.12", "3.13", "3.14", "3.15")


class CompatibilityCommandTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("check-cpython-compatibility", "ruff-corpus-expectations"):
            shutil.copy2(SCRIPTS / name, scripts / name)
        self.pin_check = scripts / "check-ruff-pin"
        self.pin_check.write_text("#!/usr/bin/env bash\nexit 0\n")
        self.pin_check.chmod(0o755)
        self.roots = [
            self.root / "vendor/ruff" / suffix
            for suffix in (
                "crates/ty_python_semantic/resources/corpus",
                "crates/ruff_python_parser/resources/valid",
                "crates/ruff_python_parser/resources/inline/ok",
                "crates/ruff_linter/resources/test/fixtures",
                "crates/ruff_python_formatter/resources/test/fixtures",
            )
        ]
        for root in self.roots:
            root.mkdir(parents=True)
        executable = self.root / "cargo"
        executable.write_text(
            f"#!{sys.executable}\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n"
        )
        executable.chmod(0o755)
        self.build_record = self.root / "build.json"
        self.python = self.root / "python with spaces"
        self.python.write_text(
            f"#!{sys.executable}\nimport json, os, pathlib, sys\n"
            "sys.exit(8) if os.environ.get('FAIL_REFERENCE_BUILD') else None\n"
            f"pathlib.Path({str(self.build_record)!r}).write_text(json.dumps(sys.argv))\n"
            "pathlib.Path(sys.argv[-1]).write_text('reference placeholder')\n"
        )
        self.python.chmod(0o755)
        shutil.copy2(self.python, self.root / "python3.14")
        self.environment = {"PATH": str(self.root) + os.pathsep + os.defpath}

    def invoke(self, *arguments, **environment):
        return subprocess.run(
            [
                "bash",
                str(self.root / "scripts/check-cpython-compatibility"),
                *arguments,
            ],
            env=self.environment | environment,
            capture_output=True,
            text=True,
            check=False,
        )

    def comparison(self, *arguments, **environment):
        result = self.invoke(*arguments, **environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        command = json.loads(result.stdout)
        self.assertEqual(
            command[:7],
            [
                "run",
                "--locked",
                "--manifest-path",
                str(self.root / "Cargo.toml"),
                "-p",
                "serc",
                "--example",
            ],
        )
        self.assertEqual(command[7:9], ["compare_cpython_corpus", "--"])
        self.assertEqual(command[-5:], list(map(str, self.roots)))
        return command[9:-5]

    def expected(self, version):
        return subprocess.check_output(
            [str(SCRIPTS / "ruff-corpus-expectations"), version], text=True
        ).splitlines()

    def built_comparison(self, *arguments, python, **environment):
        command = self.comparison(*arguments, **environment)
        self.assertEqual(command[0], "--reference")
        self.assertEqual(
            json.loads(self.build_record.read_text()),
            [
                str(python),
                "-B",
                "-I",
                "-S",
                str(self.root / "scripts/build-cpython-reference.py"),
                "--output",
                command[1],
            ],
        )
        self.assertFalse(
            Path(command[1]).parent.exists(), "temporary reference was not removed"
        )
        return command[2:]

    def test_python_mode_builds_reference_and_preserves_every_expectation(self):
        for version in VERSIONS:
            with self.subTest(version=version):
                self.assertEqual(
                    self.built_comparison(
                        str(self.python), version, python=self.python
                    ),
                    ["--python-version", version, *self.expected(version)],
                )

    def test_reference_mode_preserves_every_numeric_gate(self):
        for version in VERSIONS:
            with self.subTest(version=version):
                expected = [
                    "--reference",
                    "/reference with spaces",
                    "--python-version",
                    version,
                ]
                expected += self.expected(version)
                self.assertEqual(
                    self.comparison(
                        "--reference",
                        "/reference with spaces",
                        version,
                        CPYTHON="/not/the/reference",
                    ),
                    expected,
                )
                self.assertFalse(self.build_record.exists())

    def test_default_python_target(self):
        self.assertEqual(
            self.built_comparison(python=self.root / "python3.14")[:2],
            ["--python-version", "3.14"],
        )

    def test_cpython_environment_is_preserved(self):
        self.assertEqual(
            self.built_comparison(CPYTHON=str(self.python), python=self.python)[:2],
            ["--python-version", "3.14"],
        )

    def test_reference_build_failure_never_invokes_cargo(self):
        result = self.invoke(str(self.python), FAIL_REFERENCE_BUILD="1")
        self.assertEqual(result.returncode, 8)
        self.assertEqual(result.stdout, "")

    def test_default_reference_target(self):
        self.assertEqual(
            self.comparison("--reference", "/reference")[:4],
            ["--reference", "/reference", "--python-version", "3.14"],
        )

    def test_bad_arguments_never_invoke_cargo(self):
        for arguments in (
            ("--reference",),
            ("--reference", ""),
            ("--reference", "--python"),
            ("--reference", "/ref", "3.14", "extra"),
            ("/python", "3.14", "extra"),
            ("--reference", "/ref", "3.11"),
        ):
            with self.subTest(arguments=arguments):
                result = self.invoke(*arguments)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")

    def test_pin_failure_is_not_bypassed(self):
        self.pin_check.write_text("#!/usr/bin/env bash\nexit 7\n")
        result = self.invoke("--reference", "/ref")
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, "")

    def test_missing_corpus_is_not_bypassed(self):
        self.roots[0].rmdir()
        result = self.invoke("--reference", "/ref")
        self.assertEqual(result.returncode, 2)
        self.assertIn("corpus is missing", result.stderr)
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
