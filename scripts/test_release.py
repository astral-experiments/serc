from __future__ import annotations

import os
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHECK = runpy.run_path(str(ROOT / "scripts/check-release.py"))
UPDATE = runpy.run_path(str(ROOT / "scripts/update-release-version.py"))


class ReleaseTests(unittest.TestCase):
    def test_version_update_keeps_python_library_and_cli_in_sync(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for filename in (
                "pyproject.toml",
                "crates/serc/Cargo.toml",
                "crates/serc_cli/Cargo.toml",
            ):
                target = root / filename
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / filename, target)
            (root / "CHANGELOG.md").write_text("# Changelog\n\n## 0.2.0\n\nChanges.\n")
            self.assertEqual(UPDATE["update_versions"](root), "0.2.0")
            self.assertEqual(CHECK["release_version"](root), "0.2.0")
            cli = root / "crates/serc_cli/Cargo.toml"
            cli.write_text(
                cli.read_text().replace(
                    'path = "../serc", version = "0.2.0"',
                    'path = "../serc", version = "0.1.0"',
                )
            )
            with self.assertRaisesRegex(ValueError, "Release versions disagree"):
                CHECK["release_version"](root)

    def test_publishing_requires_explicit_enablement_and_main(self):
        version = CHECK["release_version"](ROOT)
        for enabled, ref in (
            ("false", "refs/heads/main"),
            ("true", "refs/heads/feature"),
        ):
            with self.subTest(enabled=enabled, ref=ref):
                env = dict(os.environ)
                env.pop("GITHUB_OUTPUT", None)
                env.update(
                    GITHUB_ACTIONS="true",
                    GITHUB_REPOSITORY="astral-sh/serc",
                    GITHUB_REF=ref,
                    SERC_RELEASES_ENABLED=enabled,
                )
                result = subprocess.run(
                    [sys.executable, ROOT / "scripts/check-release.py", version],
                    env=env,
                    capture_output=True,
                )
                self.assertNotEqual(result.returncode, 0)
                dry_run = subprocess.run(
                    [sys.executable, ROOT / "scripts/check-release.py", "dry-run"],
                    env=env,
                    capture_output=True,
                )
                self.assertEqual(dry_run.returncode, 0, dry_run.stderr)


if __name__ == "__main__":
    unittest.main()
