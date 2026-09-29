use std::fmt::Write as _;

use crate::common;

#[test]
fn skipped_annotation_indices_cross_small_integer_boundary() {
    for count in [15, 16, 254, 255, 256, 257] {
        let mut source = String::from("while False:\n");
        for index in 0..count {
            writeln!(source, "    hidden{index}: int").unwrap();
        }
        source.push_str("else:\n    first: bytes\nvisible: str\n");
        common::assert_matches_cpython_marshal(&source, "large_annotation_indices.py");
    }
}
