"""Source-only tests for candidate coverage and mandatory frozen interns."""

from __future__ import annotations

import unittest

from generate_cpython_caller import (
    c_candidates,
    code_facts,
    decode_c_string,
    frozen_interns,
)


class CallerSourceFactsTests(unittest.TestCase):
    def test_adjacent_literals_across_comments_are_not_lost(self):
        words = c_candidates('const char *p = "combined_" /* gap */ "name";')
        self.assertIn("combined_name", words)

    def test_punctuation_and_character_literals_stop_concatenation(self):
        words = c_candidates('f("left_name", "right_name"); "a" \'x\' "b";')
        self.assertNotIn("left_nameright_name", words)
        self.assertNotIn("ab", words)

    def test_c_hex_octal_and_unicode_escapes_are_decoded(self):
        self.assertEqual(decode_c_string(r'"\x61\142\u0063\U00000064"'), "abcd")
        self.assertIn("bounded_name", c_candidates(r'"bounded\137name"'))

    def test_hex_escape_uses_all_digits(self):
        self.assertEqual(decode_c_string(r'"\x61b"'), chr(0x61B))

    def test_macro_operands_and_dotted_name_suffixes_are_candidates(self):
        words = c_candidates('EXPORT_CONSTANT(SETTING_NAME); "module.ExportedType";')
        self.assertIn("SETTING_NAME", words)
        self.assertIn("ExportedType", words)

    def test_frozen_metadata_and_word_constants_do_not_need_live_code_owner(self):
        code = compile(
            'discarded = "suite_literal"\n'
            "class Container:\n"
            "    def nested(self):\n"
            '        return ("owned_literal", "ordinary prose")\n',
            "<frozen sourcefacts>",
            "exec",
        )
        words = frozen_interns(code)
        self.assertIn("suite_literal", words)
        self.assertIn("owned_literal", words)
        self.assertIn("Container.nested", words)
        self.assertIn("<frozen sourcefacts>", words)
        self.assertNotIn("ordinary prose", words)

    def test_code_identifiers_do_not_promote_literals(self):
        code = compile(
            'def retained(argument): return "possible_only"', "module.py", "exec"
        )
        immortal, possible = code_facts(code)
        self.assertIn("argument", immortal)
        self.assertNotIn("possible_only", immortal)
        self.assertIn("possible_only", possible)


if __name__ == "__main__":
    unittest.main()
