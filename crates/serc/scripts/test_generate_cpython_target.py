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


class RenderPreinternedLookupTest(unittest.TestCase):
    def test_escapes_rust_string_literals(self) -> None:
        self.assertEqual(
            generator.rust_string_literal('quote" slash\\ \n\r\t\0 ☃\x01\x7f'),
            '"quote\\" slash\\\\ \\n\\r\\t\\0 ☃\\x01\\x7f"',
        )

    def test_renders_exact_prefix_ranges_and_startup_candidates(self) -> None:
        self.assertEqual(
            generator.render_preinterned_lookup(
                ["", "a", "alpha", "alpine", "beta"],
                ("500", "albatross", "ascii"),
            ),
            [
                "pub(crate) fn is_preinterned_string(value: &str) -> bool {",
                "    let prefix = match value.as_bytes() {",
                "        [] => return true,",
                "        [first] => u16::from(*first) << 8,",
                "        [first, second, ..] => u16::from_be_bytes([*first, *second]),",
                "    };",
                "    let (start, end) = match prefix {",
                '        0x3530 => return value == "500",',
                "        0x6100 => (1, 2),",
                "        0x616c => {",
                '            if value == "albatross" {',
                "                return true;",
                "            }",
                "            (2, 4)",
                "        }",
                '        0x6173 => return value == "ascii",',
                "        0x6265 => (4, 5),",
                "        _ => return false,",
                "    };",
                "    PREINTERNED_STRINGS[start..end].binary_search(&value).is_ok()",
                "}",
            ],
        )


if __name__ == "__main__":
    unittest.main()
