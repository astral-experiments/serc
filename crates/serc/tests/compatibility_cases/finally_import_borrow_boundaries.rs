use crate::common;

#[test]
fn matches_cpython_borrows_across_finally_import_boundaries() {
    let cases: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/finally_import_borrow_targets.json"))
            .expect("public finally import cases are filename-to-source strings");
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(&source, &filename);
    }
}

#[test]
fn preserves_cpython_borrows_at_finally_exit_boundaries() {
    let cases: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/finally_import_borrow_controls.json"))
            .expect("public finally boundary controls are filename-to-source strings");
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(&source, &filename);
    }
}

#[test]
fn matches_cpython_terminal_finally_while_exit_locations() {
    let cases: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/finally_while_exit_locations.json"))
            .expect("public finally while cases are filename-to-source strings");
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(&source, &filename);
    }
}

#[test]
fn terminating_if_arms_preserve_the_else_borrow_boundary_before_import() {
    for exit in ["return", "raise Error()"] {
        let source = format!(
            "def choose(flag, table, key):\n    if flag:\n        {exit}\n    else:\n        key = table[normalize(key)]\n    from other import result\n    return result\n"
        );
        common::assert_matches_cpython_marshal(&source, "sole_else_import.py");
    }
}
