"""Compile-only tests for version-specific positive membership guarantees."""

import unittest

from generate_cpython_caller_variants import (
    claims_module,
    frozen_positives,
    identifiers,
)


class VariantPositiveTests(unittest.TestCase):
    def setUp(self):
        self.code = compile(
            "class Outer:\n"
            "    def inner(self, argument):\n"
            '        return ("word_literal", "ordinary prose")\n',
            "<frozen sample>",
            "exec",
        )

    def test_legacy_metadata_is_not_a_positive_without_interning(self):
        result = frozen_positives(self.code, "3.12")
        self.assertIn("word_literal", result)
        self.assertIn("argument", result)
        self.assertNotIn("Outer.inner", result)
        self.assertNotIn("<frozen sample>", result)
        self.assertNotIn("ordinary prose", result)

    def test_later_frozen_metadata_is_interned_before_marshalling(self):
        for version in ("3.13", "3.15"):
            with self.subTest(version=version):
                result = frozen_positives(self.code, version)
                self.assertIn("Outer.inner", result)
                self.assertIn("<frozen sample>", result)
                self.assertNotIn("ordinary prose", result)

    def test_source_identifier_proof_does_not_promote_constant_words(self):
        result = identifiers(self.code)
        self.assertIn("argument", result)
        self.assertNotIn("word_literal", result)
        self.assertNotIn("Outer.inner", result)

    def test_best_effort_imports_do_not_create_unconditional_positives(self):
        for version in ("3.12", "3.13", "3.15"):
            self.assertFalse(claims_module("linecache", version))
        self.assertTrue(claims_module("zipimport", "3.12"))
        self.assertFalse(claims_module("zipimport", "3.13"))
        self.assertFalse(claims_module("zipimport", "3.15"))


if __name__ == "__main__":
    unittest.main()
