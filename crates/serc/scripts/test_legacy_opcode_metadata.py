"""Selection and expression regression tests for older pinned opcode formats."""

from __future__ import annotations

import unittest

from legacy_opcode_metadata import (
    _select_opcodes,
    parse_312_metadata,
    parse_313_metadata,
    rust_stack_expression,
)


def fixture_sources(minor: int) -> dict[str, str]:
    opcodes = {
        "CACHE": 0,
        "RETURN_CONST": 121,
        "FOR_ITER": 93,
        "RESUME": 151,
        "LOAD_CONST": 100,
        "POP_JUMP_IF_FALSE": 114,
        "JUMP_FORWARD": 110,
        "INSTRUMENTED_RESUME": 240,
    }
    # A specialized instruction can occupy a hole below the normal opcodes.
    header = "\n".join(f"#define {name} {code}" for name, code in opcodes.items())
    header += "\n#define BINARY_OP_ADD_INT 6\n"
    metadata = ""
    for direction in ("popped", "pushed"):
        jump = ", bool jump" if minor == 312 else ""
        metadata += f"int _PyOpcode_num_{direction}(int opcode, int oparg{jump}) {{\n"
        metadata += "\n".join(f"case {name}: return 0;" for name in opcodes)
        metadata += "\n}\n"
    metadata += "\n".join(
        f"[{name}] = {{ true, INSTR_FMT_IB, 0 }}," for name in opcodes
    )
    caches = "const uint8_t _PyOpcode_Caches[256] = {\n[FOR_ITER] = 1,\n};"
    sources = {
        "Include/ceval.h": "#define FVS_MASK 0x4\n#define FVS_HAVE_SPEC 0x4\n",
        "Include/internal/pycore_opcode_utils.h": (
            "#define IS_SCOPE_EXIT_OPCODE(opcode) \\\n ((opcode) == RETURN_CONST)\n"
            "#define IS_UNCONDITIONAL_JUMP_OPCODE(opcode) \\\n ((opcode) == JUMP_FORWARD)\n"
        ),
    }
    if minor == 312:
        python = "HAVE_ARGUMENT = 90\nMIN_INSTRUMENTED_OPCODE = 237\n"
        python += "\n".join(
            f"def_op({name!r}, {code})" for name, code in opcodes.items()
        )
        python += (
            "\nhasarg.extend([op for op in opmap.values() if op >= HAVE_ARGUMENT])\n"
        )
        python += "hasconst.append(100)\nhasconst.append(121)\nhasjrel.append(93)\n"
        sources.update(
            {
                "Lib/opcode.py": python,
                "Include/opcode.h": header,
                "Python/opcode_metadata.h": metadata,
                "Include/internal/pycore_opcode.h": caches,
            }
        )
    else:
        sources.update(
            {
                "Lib/_opcode_metadata.py": f"opmap = {opcodes!r}\nMIN_INSTRUMENTED_OPCODE = 237\n",
                "Include/opcode_ids.h": header,
                "Include/internal/pycore_opcode_metadata.h": metadata + "\n" + caches,
            }
        )
    return sources


class LegacyMetadataTests(unittest.TestCase):
    def test_312_keeps_argument_opcodes_and_excludes_specialized_holes(self):
        result = parse_312_metadata(fixture_sources(312))
        self.assertIn(("RESUME", 151), result["opcodes"])
        self.assertIn(("LOAD_CONST", 100), result["opcodes"])
        self.assertNotIn(("BINARY_OP_ADD_INT", 6), result["opcodes"])
        self.assertNotIn(("INSTRUMENTED_RESUME", 240), result["opcodes"])
        self.assertEqual(
            result["flags"]["LOAD_CONST"], {"HAS_ARG_FLAG", "HAS_CONST_FLAG"}
        )
        self.assertEqual(result["caches"], {"FOR_ITER": 1})
        self.assertEqual(
            result["predicates"]["is_conditional_jump"], {"POP_JUMP_IF_FALSE"}
        )

    def test_313_selects_literal_opmap_instead_of_a_numeric_cutoff(self):
        result = parse_313_metadata(fixture_sources(313))
        self.assertIn(("RESUME", 151), result["opcodes"])
        self.assertNotIn(("BINARY_OP_ADD_INT", 6), result["opcodes"])

    def test_conflicting_generated_header_is_rejected(self):
        sources = fixture_sources(312)
        sources["Include/opcode.h"] = sources["Include/opcode.h"].replace(
            "LOAD_CONST 100", "LOAD_CONST 101"
        )
        with self.assertRaisesRegex(ValueError, "disagrees"):
            parse_312_metadata(sources)

    def test_missing_stack_metadata_is_rejected(self):
        sources = fixture_sources(312)
        sources["Python/opcode_metadata.h"] = sources[
            "Python/opcode_metadata.h"
        ].replace("case FOR_ITER: return 0;", "")
        with self.assertRaisesRegex(ValueError, "missing legacy popped stack effects"):
            parse_312_metadata(sources)

    def test_header_aliases_cannot_create_duplicate_opcode_ids(self):
        with self.assertRaisesRegex(ValueError, "duplicate physical opcode"):
            _select_opcodes(
                {
                    "CACHE": 0,
                    "RESUME": 151,
                    "FOR_ITER": 93,
                    "RETURN_CONST": 121,
                    "ALIAS": 93,
                },
                "",
                237,
            )

    def test_old_ternaries_keep_integer_truth_and_nested_bit_masks(self):
        self.assertEqual(
            rust_stack_expression("((oparg & 1) ? 1 : 0) + 3"),
            "i64::from(argument & 1 != 0) + 3",
        )
        self.assertEqual(
            rust_stack_expression("(((oparg & 4) == 4) ? 1 : 0) + 1"),
            "i64::from(argument & 4 == 4) + 1",
        )
        self.assertEqual(
            rust_stack_expression("2 + ((oparg == 3) ? 1 : 0)"),
            "2 + i64::from(argument == 3)",
        )

    def test_unknown_or_executable_stack_expressions_are_rejected(self):
        for expression in (
            "jump",
            "FVS_MASK",
            "__import__('os')",
            "oparg.foo",
            "int(oparg, 10)",
        ):
            with self.subTest(expression=expression), self.assertRaises(ValueError):
                rust_stack_expression(expression)


if __name__ == "__main__":
    unittest.main()
