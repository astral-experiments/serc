from __future__ import annotations

import json
import runpy
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

CHECK = runpy.run_path(str(Path(__file__).with_name("check-ruff-pin")))
VERSION = "0.0.15"


class RuffPinTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.ruff = self.root / "vendor/ruff"
        self.ruff.mkdir(parents=True)
        for root in (self.root, self.ruff):
            self.git(root, "init", "--quiet")
            self.git(root, "config", "user.name", "Serc Tests")
            self.git(root, "config", "user.email", "serc@example.com")
            self.git(root, "config", "commit.gpgSign", "false")
        (self.ruff / "README.md").write_text("Ruff fixture\n")
        self.git(self.ruff, "add", ".")
        self.git(self.ruff, "commit", "--quiet", "-m", "fixture")
        self.revision = self.git(self.ruff, "rev-parse", "HEAD")
        self.git(
            self.root,
            "update-index",
            "--add",
            "--cacheinfo",
            f"160000,{self.revision},vendor/ruff",
        )
        self.packages = []
        for name in ["ruff_python_ast", "ruff_python_parser", "ruff_text_size"]:
            source = self.root / "registry" / name
            source.mkdir(parents=True)
            (source / ".cargo_vcs_info.json").write_text(
                json.dumps({"git": {"sha1": self.revision}})
            )
            self.packages.append(
                {
                    "name": name,
                    "version": VERSION,
                    "source": CHECK["CRATES_IO"],
                    "manifest_path": str(source / "Cargo.toml"),
                }
            )

    def git(self, root, *args):
        return subprocess.check_output(["git", "-C", root, *args], text=True).strip()

    def check(self):
        revision = CHECK["check_corpus"](self.root)
        with patch.dict(
            CHECK["check_dependencies"].__globals__,
            cargo_metadata=lambda root: {"packages": self.packages},
        ):
            CHECK["check_dependencies"](self.root, revision)

    def test_coherent_inputs(self):
        self.check()

    def test_stale_version(self):
        self.packages[0]["version"] = "0.0.14"
        with self.assertRaisesRegex(ValueError, "must resolve to one release"):
            self.check()

    def test_git_or_path_dependency(self):
        for source in (None, "git+https://github.com/astral-sh/ruff"):
            with self.subTest(source=source):
                self.packages[0]["source"] = source
                with self.assertRaisesRegex(ValueError, "must resolve to crates.io"):
                    self.check()

    def test_missing_dependencies(self):
        self.packages.clear()
        with self.assertRaisesRegex(ValueError, "no Ruff dependencies"):
            self.check()

    def test_published_revision_mismatch_in_transitive_dependency(self):
        vcs = Path(self.packages[-1]["manifest_path"]).with_name(".cargo_vcs_info.json")
        for git in ({"sha1": "1" * 40}, {"sha1": self.revision, "dirty": True}):
            with self.subTest(git=git):
                vcs.write_text(json.dumps({"git": git}))
                with self.assertRaisesRegex(ValueError, "published source revision"):
                    self.check()

    def test_uninitialized_submodule(self):
        (self.ruff / ".git").rename(self.root / "saved-git")
        with self.assertRaisesRegex(ValueError, "submodule is not initialized"):
            self.check()

    def test_submodule_head_mismatch(self):
        self.git(self.ruff, "commit", "--allow-empty", "--quiet", "-m", "new revision")
        with self.assertRaisesRegex(ValueError, "HEAD does not match"):
            self.check()

    def test_local_corpus_changes(self):
        readme = self.ruff / "README.md"
        readme.write_text("modified fixture\n")
        with self.assertRaisesRegex(ValueError, "local changes"):
            self.check()
        self.git(self.ruff, "restore", "README.md")
        (self.ruff / "untracked.py").write_text("pass\n")
        with self.assertRaisesRegex(ValueError, "local changes"):
            self.check()


if __name__ == "__main__":
    unittest.main()
