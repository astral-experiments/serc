from __future__ import annotations

import copy
import tempfile
import unittest
from pathlib import Path

import mypy_primer_corpus as corpus


class MypyPrimerCorpusTests(unittest.TestCase):
    def git(self, checkout: Path, *arguments: str) -> str:
        return corpus.git(list(arguments), cwd=checkout).stdout.strip()

    def initialize(self, checkout: Path) -> None:
        self.git(checkout, "init", "--quiet")
        self.git(checkout, "config", "user.name", "Serc Tests")
        self.git(checkout, "config", "user.email", "serc@example.com")
        self.git(checkout, "config", "commit.gpgSign", "false")

    def commit(self, checkout: Path, message: str) -> str:
        self.git(checkout, "add", ".")
        self.git(checkout, "commit", "--quiet", "-m", message)
        return self.git(checkout, "rev-parse", "HEAD")

    def test_catalog_is_read_statically_and_deduplicated_by_update(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            checkout = Path(temporary)
            self.initialize(checkout)
            package = checkout / "mypy_primer"
            package.mkdir()
            (package / "projects.py").write_text(
                """
def get_projects():
    projects = [
        Project(location="https://github.com/example/project"),
        Project(
            location="https://github.com/example/project",
            name_override="second logical project",
        ),
    ]
    return projects
""",
                encoding="utf-8",
            )
            revision = self.commit(checkout, "catalog")

            actual_revision, entries = corpus.read_catalog(checkout)

            self.assertEqual(actual_revision, revision)
            self.assertEqual(
                entries,
                [
                    ("https://github.com/example/project", "project"),
                    ("https://github.com/example/project", "second logical project"),
                ],
            )

            (package / "projects.py").write_text(
                """
def get_projects():
    projects = [project_factory()]
    return projects
""",
                encoding="utf-8",
            )
            self.commit(checkout, "dynamic catalog")
            with self.assertRaisesRegex(corpus.CorpusError, "static list"):
                corpus.read_catalog(checkout)

    def test_locked_checkout_uses_exact_revision_and_only_python_sources(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            upstream = root / "upstream"
            upstream.mkdir()
            self.initialize(upstream)
            (upstream / "module.py").write_text("first = 1\n", encoding="utf-8")
            (upstream / "module.pyi").write_text("first: int\n", encoding="utf-8")
            (upstream / "ignored.txt").write_text("not source\n", encoding="utf-8")
            locked_revision = self.commit(upstream, "locked")
            (upstream / "module.py").write_text("second = 2\n", encoding="utf-8")
            self.commit(upstream, "new head")

            repository = {
                "id": "example",
                "url": str(upstream),
                "revision": locked_revision,
                "logical_projects": ["example"],
            }
            corpus_root = root / "corpus"

            corpus.setup_repository(corpus_root, repository)

            checkout = corpus.checkout_path(corpus_root, repository)
            self.assertEqual(corpus.current_revision(checkout), locked_revision)
            self.assertEqual(
                (checkout / "module.py").read_text(encoding="utf-8"), "first = 1\n"
            )
            self.assertTrue((checkout / "module.pyi").is_file())
            self.assertFalse((checkout / "ignored.txt").exists())

            lock = {"repositories": [repository]}
            expected = corpus.inspect_content(corpus_root, lock["repositories"])
            lock["content"] = expected
            self.assertEqual(corpus.verify_checkouts(corpus_root, lock), expected)

            (checkout / "generated.py").write_text("dirty = True\n", encoding="utf-8")
            with self.assertRaisesRegex(corpus.CorpusError, "local changes"):
                corpus.verify_checkouts(corpus_root, lock)

    def test_lock_validation_rejects_duplicate_urls_and_short_revisions(self) -> None:
        base = {
            "format": 1,
            "catalog": {
                "url": corpus.CATALOG_URL,
                "revision": "1" * 40,
                "entries": 1,
                "repositories": 1,
            },
            "resolved_at": "2026-07-07T00:00:00+00:00",
            "source_suffixes": [".py", ".pyi"],
            "content": None,
            "repositories": [
                {
                    "id": "first",
                    "url": "https://github.com/example/project",
                    "revision": "2" * 40,
                    "logical_projects": ["project"],
                }
            ],
        }
        corpus.validate_lock(base)

        duplicate = copy.deepcopy(base)
        duplicate["repositories"].append(
            {
                "id": "second",
                "url": "https://github.com/example/project",
                "revision": "3" * 40,
                "logical_projects": ["other project"],
            }
        )
        duplicate["catalog"]["entries"] = 2
        duplicate["catalog"]["repositories"] = 2
        with self.assertRaisesRegex(corpus.CorpusError, "duplicate repository URL"):
            corpus.validate_lock(duplicate)

        short = copy.deepcopy(base)
        short["repositories"][0]["revision"] = "abc"
        with self.assertRaisesRegex(corpus.CorpusError, "invalid revision"):
            corpus.validate_lock(short)

    def test_compatibility_command_freezes_the_full_failure_count(self) -> None:
        roots = [Path("/corpus/first"), Path("/corpus/second")]
        full = corpus.compatibility_command("python3.14", roots, 73_537, None)
        expected_exact_index = full.index("--expected-exact") + 1
        rejected_index = full.index("--expected-cpython-rejected") + 1
        expected_exact = int(full[expected_exact_index])
        expected_rejected = int(full[rejected_index])

        self.assertEqual(expected_exact, 66_246)
        self.assertEqual(expected_rejected, 160)
        self.assertEqual(
            73_537 - expected_exact - expected_rejected, corpus.EXPECTED_FAILURES
        )
        self.assertNotIn("--require-all", full)

        limited = corpus.compatibility_command("python3.14", roots, 73_537, 100)
        self.assertNotIn("--expected-exact", limited)
        self.assertNotIn("--expected-cpython-rejected", limited)
        self.assertEqual(limited[limited.index("--limit") + 1], "100")
        self.assertIn("--require-all", limited)


if __name__ == "__main__":
    unittest.main()
