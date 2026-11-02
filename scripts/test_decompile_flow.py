from __future__ import annotations

import unittest

from decompile_flow import FIELDS, function_matches


def check(original, candidate, *, optimize=0):
    filename = "example.py"
    before = compile(original, filename, "exec", dont_inherit=True, optimize=optimize)
    after = compile(candidate, filename, "exec", dont_inherit=True, optimize=0)
    return function_matches(before, after, candidate.encode(), filename)


class FunctionFlowTests(unittest.TestCase):
    def test_checks_flow_and_table_independently(self):
        original = (
            "def choose(x):\n    if x:\n        raise LookupError\n    return 1\n"
        )
        self.assertEqual(
            check(original, original.replace("LookupError", "KeyError")),
            {("choose", 1): FIELDS},
        )
        self.assertEqual(
            check(original, original.replace("raise LookupError", "return 2")),
            {("choose", 1): {"exception_table"}},
        )
        handler = "def run(f):\n    try:\n        f()\n    except ValueError:\n        return\n"
        padded = handler.replace("        f()", "        f()\n        f()")
        self.assertEqual(check(handler, padded), {("run", 1): {"normal_flow"}})

    def test_matches_the_input_optimization_level(self):
        source = "def positive(x):\n    assert x > 0\n    return x\n"
        self.assertEqual(check(source, source, optimize=1), {("positive", 1): FIELDS})

    def test_identifies_placed_definitions_and_real_gap_comments(self):
        source = '''@decorate
def kept():
    """# serc: unrecovered body"""
    ...

def partial():
    ...  # serc: unrecovered body of partial: bad branch
'''
        self.assertEqual(check(source, source), {("kept", 1): FIELDS})

    def test_repeated_definitions_must_pair_within_their_parent(self):
        source = """class C:
    def f(self):
        return 1
class C:
    def f(self):
        return 2
"""
        self.assertEqual(
            check(source, source), {("C.f", 1): FIELDS, ("C.f", 2): FIELDS}
        )
        self.assertEqual(check(source, source[: source.rindex("class C:")]), {})


if __name__ == "__main__":
    unittest.main()
