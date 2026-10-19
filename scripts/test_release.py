from __future__ import annotations

import os
import runpy
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHECK = runpy.run_path(str(ROOT / "scripts/check-release.py"))
UPDATE = runpy.run_path(str(ROOT / "scripts/update-release-version.py"))


class ReleaseTests(unittest.TestCase):
    def test_rewrite_latest_changelog_section(self):
        script = ROOT / "scripts/update-latest-changelog-section.py"
        for history in ("", "\n## 0.1.0\n\nOlder release.\n"):
            with (
                self.subTest(history=history),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                changelog = root / "CHANGELOG.md"
                candidate = root / "candidate.md"
                changelog.write_text(
                    "# Changelog\n\n## 0.2.0\n\nGenerated.\n" + history
                )
                candidate.write_text("## 0.2.0\n\nRewritten.\n")
                subprocess.run(
                    [sys.executable, script, changelog, candidate], check=True
                )
                self.assertEqual(
                    changelog.read_text(),
                    "# Changelog\n\n## 0.2.0\n\nRewritten.\n" + history,
                )

    def test_version_update_keeps_python_library_cli_and_dist_in_sync(self):
        for version, python_version in (
            ("0.2.0", "0.2.0"),
            ("0.2.0-alpha.1", "0.2.0a1"),
            ("0.2.0-beta.2", "0.2.0b2"),
            ("0.2.0-rc.3", "0.2.0rc3"),
            ("0.2.0-dev.4", "0.2.0.dev4"),
        ):
            with (
                self.subTest(version=version),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                for filename in (
                    "pyproject.toml",
                    "crates/serc/Cargo.toml",
                    "crates/serc_cli/Cargo.toml",
                    "dist-workspace.toml",
                ):
                    target = root / filename
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / filename, target)
                (root / "CHANGELOG.md").write_text(
                    f"# Changelog\n\n## {version}\n\nChanges.\n"
                )
                self.assertEqual(UPDATE["update_versions"](root), version)
                self.assertEqual(CHECK["release_version"](root), version)
                project = tomllib.loads((root / "pyproject.toml").read_text())[
                    "project"
                ]
                self.assertEqual(project["version"], python_version)
                dist = root / "dist-workspace.toml"
                dist.write_text(dist.read_text().replace(f'"{version}"', '"0.1.0"'))
                with self.assertRaisesRegex(ValueError, "Release versions disagree"):
                    CHECK["release_version"](root)
                UPDATE["update_versions"](root)
                cli = root / "crates/serc_cli/Cargo.toml"
                cli.write_text(
                    cli.read_text().replace(
                        f'path = "../serc", version = "{version}"',
                        'path = "../serc", version = "0.1.0"',
                    )
                )
                with self.assertRaisesRegex(ValueError, "Release versions disagree"):
                    CHECK["release_version"](root)

    def test_publishing_requires_serc_main(self):
        version = CHECK["release_version"](ROOT)
        for repository, ref, allowed in (
            ("astral-sh/serc", "refs/heads/main", True),
            ("astral-sh/serc", "refs/heads/feature", False),
            ("example/serc", "refs/heads/main", False),
        ):
            with self.subTest(repository=repository, ref=ref):
                env = dict(os.environ)
                env.pop("GITHUB_OUTPUT", None)
                env.update(
                    GITHUB_ACTIONS="true",
                    GITHUB_REPOSITORY=repository,
                    GITHUB_REF=ref,
                )
                result = subprocess.run(
                    [sys.executable, ROOT / "scripts/check-release.py", version],
                    env=env,
                    capture_output=True,
                )
                self.assertEqual(result.returncode == 0, allowed, result.stderr)
                dry_run = subprocess.run(
                    [sys.executable, ROOT / "scripts/check-release.py", "dry-run"],
                    env=env,
                    capture_output=True,
                )
                self.assertEqual(dry_run.returncode, 0, dry_run.stderr)


if __name__ == "__main__":
    unittest.main()
