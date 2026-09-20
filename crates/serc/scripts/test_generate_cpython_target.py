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
    def test_parses_only_compiler_initialized_identifiers(self) -> None:
        source = """
        static int init_identifiers(struct ast_state *state)
        {
            state->right = PyUnicode_InternFromString("right");
            state->body = PyUnicode_InternFromString("body");
            state->left = PyUnicode_InternFromString("left");
            state->left = PyUnicode_InternFromString("left");
            return 0;
}
        PyUnicode_InternFromString("unrelated");
        """
        self.assertEqual(
            generator.parse_compiler_interned_strings(source),
            ["body", "left", "right"],
        )
        with self.assertRaisesRegex(ValueError, "incomplete"):
            generator.parse_compiler_interned_strings(
                source.replace('PyUnicode_InternFromString("body")', "NULL")
            )

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

    def test_gates_unix_startup_candidates(self) -> None:
        rendered = generator.render_preinterned_lookup(
            ["package"], ("patterns", "pause")
        )
        self.assertIn(
            '            if value == "patterns" || (cfg!(unix) && value == "pause") {',
            rendered,
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


class TargetSelectionTest(unittest.TestCase):
    def tearDown(self) -> None:
        generator.select_target_version("3.14")

    def test_selects_exact_release_and_protocol_inputs(self) -> None:
        generator.select_target_version("3.15")
        self.assertEqual(generator.TARGET_TAG, "v3.15.0rc2")
        self.assertEqual(
            generator.TARGET_COMMIT, "435c9e5a798c99653e3ab64ce29baed0e4f3dfee"
        )
        self.assertEqual(generator.TARGET_VERSION, (3, 15, 0))
        self.assertEqual(generator.OUTPUT.name, "cpython_3_15_0rc2.rs")
        self.assertEqual(generator.MARSHAL_OUTPUT.name, "v6.rs")
        self.assertIn("Include/cpython/marshal.h", generator.MARSHAL_INPUTS)
        self.assertIn("Python/assemble.c", generator.TARGET_INPUTS)
        generator.select_target_version("3.14")
        self.assertEqual(generator.TARGET_TAG, "v3.14.5")
        self.assertEqual(generator.TARGET_VERSION, (3, 14, 5))
        self.assertEqual(generator.OUTPUT.name, "cpython_3_14_5.rs")
        self.assertEqual(generator.MARSHAL_OUTPUT.name, "v5.rs")
        self.assertNotIn("Python/assemble.c", generator.TARGET_INPUTS)
        self.assertEqual(generator.MARSHAL_INPUTS[0], "Include/marshal.h")

    def test_startup_fixture_matches_target_and_pinned_digest(self) -> None:
        generator.select_target_version("3.15")
        strings = generator.read_startup_strings()
        self.assertEqual(len(strings), 2790)
        self.assertEqual(strings, tuple(sorted(set(strings))))
        self.assertFalse(any(value.startswith("/") for value in strings))
        with (
            mock.patch.object(generator, "STARTUP_315_SHA256", "0" * 64),
            self.assertRaisesRegex(ValueError, "checksum mismatch"),
        ):
            generator.read_startup_strings()
        with (
            mock.patch.object(generator, "TARGET_COMMIT", "0" * 40),
            self.assertRaisesRegex(ValueError, "does not match"),
        ):
            generator.read_startup_strings()

    def test_opcode_boundary_comes_from_pinned_source(self) -> None:
        source = """
        /* Instruction opcodes for compiled code */
#define CACHE 0
#define RESUME 129
#define SPECIALIZED 130
#define HAVE_ARGUMENT 42
#define MIN_SPECIALIZED_OPCODE 130
        """
        self.assertEqual(
            generator.parse_opcodes(source), [("CACHE", 0), ("RESUME", 129)]
        )
        with self.assertRaisesRegex(ValueError, "first specialized opcode"):
            generator.parse_opcodes(source.replace("MIN_SPECIALIZED_OPCODE", "MISSING"))

    def test_import_modes_come_from_each_codegen_branch(self) -> None:
        source = """
        if (s->v.Import.is_lazy) {
            ADDOP_NAME_CUSTOM(c, loc, IMPORT_NAME, alias->name, names, 3, 1);
        } else {
            if (force_eager) {
                // force eager import in try/except block
                ADDOP_NAME_CUSTOM(c, loc, IMPORT_NAME, alias->name, names, 3, 2);
            } else {
                ADDOP_NAME_CUSTOM(c, loc, IMPORT_NAME, alias->name, names, 3, 0);
            }
        }
        """
        self.assertEqual(
            generator.parse_import_modes(source),
            (3, [("LAZY", 1), ("EAGER", 2), ("DEFAULT", 0)]),
        )
        with self.assertRaisesRegex(ValueError, "disagree"):
            generator.parse_import_modes(source.replace("names, 3, 0", "names, 2, 0"))
        with self.assertRaisesRegex(ValueError, "distinct"):
            generator.parse_import_modes(source.replace("names, 3, 0", "names, 3, 2"))

    def test_metadata_array_length_comes_from_pinned_source(self) -> None:
        source = """
const struct opcode_metadata _PyOpcode_opcode_metadata[271] = {
    [RESUME] = { true, INSTR_FMT_IBC, HAS_ARG_FLAG | HAS_EVAL_BREAK_FLAG },
};
        """
        self.assertEqual(
            generator.parse_opcode_flags(source),
            {"RESUME": {"HAS_ARG_FLAG", "HAS_EVAL_BREAK_FLAG"}},
        )


if __name__ == "__main__":
    unittest.main()
