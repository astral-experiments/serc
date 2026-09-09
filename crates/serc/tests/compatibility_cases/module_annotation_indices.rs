use std::fmt::Write as _;

use crate::common;

#[test]
fn skipped_module_annotations_reserve_registration_indices() {
    for source in [
        "if flag:\n    if False:\n        hidden: int\n    visible: str\nafter: bytes\n",
        "while False:\n    hidden: int\nelse:\n    first: bytes\nvisible: str\n",
        "for x in items:\n    if False:\n        hidden: int\n    first: bytes\nvisible: str\n",
        "with manager:\n    if False:\n        hidden: int\n    first: bytes\nvisible: str\n",
        "match value:\n    case 0 if False:\n        hidden: int\n    case _:\n        first: bytes\nvisible: str\n",
        "if flag:\n    raise Error\n    hidden: int\nvisible: str\n",
        "if flag:\n    assert False\n    hidden: int\nvisible: str\n",
    ] {
        common::assert_matches_cpython_marshal(source, "module_annotation_indices.py");
    }
}

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

#[test]
fn annotation_indices_preserve_scope_and_cleanup_visitation() {
    for source in [
        "first: int\nsecond: str\n",
        "from __future__ import annotations\nfirst: int\nsecond: str\n",
        "class C:\n    first: int\n    second: str\n",
        "def f():\n    first: int\n    second: str\n",
        "def inner():\n    hidden: int\nclass Inner:\n    local: int\nvisible: str\n",
        "if False:\n    hidden: int\n",
        "first: str\ntry:\n    work()\nfinally:\n    cleanup: bytes\nvisible: str\n",
        "first: str\ntry:\n    hidden: int\nfinally:\n    if flag:\n        cleanup: bytes\nvisible: str\n",
    ] {
        common::assert_matches_cpython_marshal(source, "annotation_scope_indices.py");
    }
}
