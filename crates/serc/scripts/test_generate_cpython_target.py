from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("generate_cpython_target.py")
SPEC = importlib.util.spec_from_file_location("generate_cpython_target", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generator)


def git(checkout: Path, *arguments: str) -> str:
    return subprocess.run(
        ["git", "-C", str(checkout), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout.rstrip("\n")


class ReadInputsTest(unittest.TestCase):
    def test_parses_preinterned_strings(self) -> None:
        source = """
        STRUCT_FOR_STR(anon_module, "<module>")
        STRUCT_FOR_STR(empty, "")
        STRUCT_FOR_ID(sort)
        STRUCT_FOR_ID(alpha)
        STRUCT_FOR_ID(sort)
        """
        self.assertEqual(
            generator.parse_preinterned_strings(source),
            ["", "<module>", "alpha", "sort"],
        )

    def test_reads_pinned_commit_instead_of_dirty_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            git(checkout, "init", "--quiet")

            expected = {}
            for relative in generator.TARGET_INPUTS + generator.MARSHAL_INPUTS:
                source = f"committed contents of {relative}\n"
                expected[relative] = source
                path = checkout / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)

            git(checkout, "add", ".")
            git(
                checkout,
                "-c",
                "user.name=Serc test",
                "-c",
                "user.email=serc-test@example.com",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            )
            commit = git(checkout, "rev-parse", "HEAD^{commit}")
            dirty_input = generator.TARGET_INPUTS[0]
            (checkout / dirty_input).write_text("dirty working-tree contents\n")

            with mock.patch.object(generator, "TARGET_COMMIT", commit):
                sources = generator.read_inputs(checkout)

            self.assertEqual(sources, expected)
            self.assertEqual(git(checkout, "rev-parse", "HEAD^{commit}"), commit)
            self.assertEqual(git(checkout, "status", "--short"), f" M {dirty_input}")


if __name__ == "__main__":
    unittest.main()
