from __future__ import annotations

import unittest

from decompile_fragments import verified_fragments


def compare(original, damaged):
    return verified_fragments(
        damaged.encode(),
        "sample.py",
        compile(original, "sample.py", "exec", dont_inherit=True),
    )


class FragmentTests(unittest.TestCase):
    def test_methods_and_inner_functions_keep_their_qualified_names(self):
        source = """\
class C:
    def pick(self, op):
        return op or (lambda: 1)
    def wrong(self):
        return 42

def complete():
    return None
"""
        broken = "1 = 2\n" + source.replace("return 42", "return 41")
        self.assertEqual(
            compare(source, broken),
            {("C.pick", 1), ("C.pick.<locals>.<lambda>", 1), ("complete", 1)},
        )

    def test_multiline_header_and_body_are_one_fragment(self):
        source = "class C:\n    def result(\n        self,\n    ):\n        return (\n            42\n        )\n"
        self.assertEqual(compare(source, "1 = 2\n" + source), {("C.result", 1)})

    def test_quoted_text_and_ambiguous_names_do_not_count(self):
        source = "def pretend(): return 1\ndef repeat(): pass\ndef repeat(): pass\n"
        broken = "1 = 2\ntext = '''\ndef pretend(): return 1\n'''\ndef repeat(): pass\n"
        self.assertEqual(compare(source, broken), set())

    def test_wrappers_supply_neither_method_bodies_nor_compiler_flags(self):
        source = "from __future__ import annotations\nclass C:\n    async def f(self): return 1\n"
        broken = source.replace("class C:", "1 = 2\nclass C(object):")
        self.assertEqual(compare(source, broken), {("C.f", 1)})
        self.assertEqual(compare(source, broken.split("\n", 1)[1]), set())


if __name__ == "__main__":
    unittest.main()
