"""Safety checks for source-codec intern metadata derivation."""

from __future__ import annotations

import unittest

import generate_source_codec_interns as generator


class ClinicMethodTests(unittest.TestCase):
    def test_rejects_static_flag_on_a_continuation_line(self):
        definition = """#define _MULTIBYTECODEC_EXAMPLE_METHODDEF \\
    {"example", example_func, METH_FASTCALL | \\
     METH_STATIC, example_doc},
"""
        with self.assertRaisesRegex(ValueError, "static method names"):
            generator.clinic_method_definitions(definition)

    def test_reads_nonstatic_method_across_continuation_lines(self):
        definition = """#define _MULTIBYTECODEC_EXAMPLE_METHODDEF \\
    {"example", example_func, METH_FASTCALL | \\
     METH_KEYWORDS, example_doc},
"""
        self.assertEqual(
            generator.clinic_method_definitions(definition),
            {"_MULTIBYTECODEC_EXAMPLE_METHODDEF": "example"},
        )


if __name__ == "__main__":
    unittest.main()
