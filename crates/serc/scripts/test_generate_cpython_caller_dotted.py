"""Source/compile-only checks for candidate coverage, not absence approval.

The snippets are parsed and compiled, never executed or imported as modules.
"""

from __future__ import annotations

import ast
import sys
import types
import unittest

from caller_dotted_metadata import lexical_qualnames
from generate_cpython_caller_dotted import DOTTED, c_literals


def code_qualnames(code):
    """Read code metadata without executing the compiled source."""
    result = {code.co_qualname}
    for value in code.co_consts:
        if isinstance(value, types.CodeType):
            result.update(code_qualnames(value))
    return result


class DottedMetadataTests(unittest.TestCase):
    def assert_compiled_names_covered(self, source, candidates):
        code = compile(source, "<candidate coverage>", "exec")
        eligible = {name for name in code_qualnames(code) if DOTTED.fullmatch(name)}
        self.assertLessEqual(eligible, candidates)
        return eligible

    def test_discarded_nested_code_remains_a_lexical_candidate(self):
        source = (
            "def outer():\n"
            "    if False:\n"
            "        def discarded():\n"
            "            def leaf():\n"
            "                return 1\n"
            "            return leaf\n"
            "    return None\n"
        )
        candidates = lexical_qualnames(ast.parse(source))
        self.assertIn("outer.<locals>.discarded", candidates)
        self.assertIn("outer.<locals>.discarded.<locals>.leaf", candidates)
        surviving = self.assert_compiled_names_covered(source, candidates)
        # CPython 3.13+ remove_unused_consts drops this assembled child.
        if sys.version_info >= (3, 13):
            self.assertNotIn("outer.<locals>.discarded", surviving)

    def test_global_reset_preserves_raw_private_name(self):
        source = (
            "class Container:\n"
            "    def __method(self):\n"
            "        global __factory\n"
            "        def __factory():\n"
            "            def leaf():\n"
            "                return 1\n"
            "            return leaf\n"
            "        return __factory\n"
        )
        candidates = lexical_qualnames(ast.parse(source))
        self.assertIn("Container.__method", candidates)
        self.assertIn("__factory.<locals>.leaf", candidates)
        self.assertIn("Container.__method.<locals>.__factory.<locals>.leaf", candidates)
        self.assertNotIn("Container._Container__method", candidates)
        self.assertNotIn("_Container__factory.<locals>.leaf", candidates)
        self.assert_compiled_names_covered(source, candidates)

    def test_annotation_helper_has_no_extra_locals_segment(self):
        source = (
            "class Container:\n"
            "    def method(self, value: int) -> str:\n"
            "        return str(value)\n"
        )
        candidates = lexical_qualnames(ast.parse(source))
        self.assertIn("Container.method.__annotate__", candidates)
        surviving = self.assert_compiled_names_covered(source, candidates)
        if sys.version_info[:2] == (3, 15):
            self.assertIn("Container.method.__annotate__", surviving)

    def test_generic_alias_and_bound_helpers_use_lexical_parent(self):
        source = (
            "class Container:\n"
            "    type Alias[T: int] = tuple[T]\n"
            "    def method[P: str](self, value: P) -> P:\n"
            "        return value\n"
        )
        candidates = lexical_qualnames(ast.parse(source))
        self.assertIn("Container.Alias", candidates)
        self.assertIn("Container.T", candidates)
        self.assertIn("Container.P", candidates)
        self.assertIn("Container.method.__annotate__", candidates)
        self.assert_compiled_names_covered(source, candidates)

    @unittest.skipIf(sys.version_info < (3, 13), "type defaults require 3.13")
    def test_type_parameter_defaults_include_raw_helper_names(self):
        source = (
            "def outer():\n"
            "    type Alias[T = int, *Ts = *tuple[int], **P = [int]] = T\n"
        )
        candidates = lexical_qualnames(ast.parse(source))
        for name in ("Alias", "T", "Ts", "P"):
            self.assertIn(f"outer.<locals>.{name}", candidates)
        self.assert_compiled_names_covered(source, candidates)


class DottedLiteralTests(unittest.TestCase):
    def test_adjacent_literals_preserve_complete_dotted_name(self):
        source = '"package." /* comment */ "Outer." \\\n "Inner"'
        literals = c_literals(source)
        self.assertIn("package.Outer.Inner", literals)
        self.assertIn("Outer.", literals)
        self.assertIn("Inner", literals)

    def test_punctuation_and_character_literals_separate_groups(self):
        literals = c_literals('f("package.", "Type"); "outer." \'x\' "Member";')
        self.assertNotIn("package.Type", literals)
        self.assertNotIn("outer.Member", literals)

    def test_dotted_grammar_only_allows_locals_angle_segment(self):
        for value in ("module.Type", "outer.<locals>.inner", "_private.Type2"):
            with self.subTest(value=value):
                self.assertIsNotNone(DOTTED.fullmatch(value))
        for value in (
            "identifier",
            ".Type",
            "module.",
            "module..Type",
            "module.<lambda>",
            "module.<genexpr>",
            "module.<generic parameters of T>.name",
            "módulo.Type",
            "module.Type\n",
        ):
            with self.subTest(value=value):
                self.assertIsNone(DOTTED.fullmatch(value))


if __name__ == "__main__":
    unittest.main()
