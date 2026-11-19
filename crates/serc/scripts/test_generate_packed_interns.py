"""Check the boundaries shared by the intern pool generator and its Rust reader."""

from __future__ import annotations

import unittest

from generate_cpython_target import rust_string_literal
from generate_packed_interns import pack_strings, string_table


class PackedInternTests(unittest.TestCase):
    def test_generated_strings_round_trip_at_byte_offsets(self):
        values = ["", "a\x0012", 'quote"\\0', "x\n\x7f", "雪", "雪"]
        literals = ",\n".join(rust_string_literal(value) for value in values)
        table = string_table(f"static WORDS: &[&str] = &[\n{literals},\n];", "WORDS")
        pool, offsets = pack_strings(set(table))
        self.assertEqual(
            [
                pool[i + 1 : i + 1 + pool[i]].decode()
                for i in map(offsets.__getitem__, table)
            ],
            values,
        )
        self.assertEqual(pool.decode().encode(), pool)

    def test_limits(self):
        pack_strings({"a" * 127})
        with self.assertRaisesRegex(ValueError, "length prefix"):
            pack_strings({"é" * 64})
        with self.assertRaisesRegex(ValueError, "offset range"):
            pack_strings({f"{i:0127d}" for i in range(513)})
