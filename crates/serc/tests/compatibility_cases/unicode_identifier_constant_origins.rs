use crate::common;

#[test]
fn annotation_and_default_keys_preserve_identifier_origin() {
    for name in ["ω", "ω_tail", "名", "é", "name"] {
        for future in ["", "from __future__ import annotations\n"] {
            for placement in [0, 1, 2] {
                for parameter in [format!("{name}: float"), format!("*, {name}=1")] {
                    let literal = format!("saved = '{name}'\n");
                    let before = if placement == 1 { &literal } else { "" };
                    let after = if placement == 2 { &literal } else { "" };
                    let source = format!(
                        "{future}{before}def previous({name}):\n    return {name}\ndef f({parameter}):\n    return {name}\n{after}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "unicode_identifier_origins.py",
                    );
                }
            }
        }
    }
}

#[test]
fn module_annotation_origins_follow_the_entire_body() {
    for name in ["ω", "ω_tail", "名", "é", "name"] {
        for future in ["", "from __future__ import annotations\n"] {
            for placement in [0, 1, 2] {
                let literal = format!("saved = '{name}'\n");
                let before = if placement == 1 { &literal } else { "" };
                let after = if placement == 2 { &literal } else { "" };
                let source = format!(
                    "{future}{before}def previous({name}):\n    return {name}\n{name}: float = 1\n{after}"
                );
                common::assert_matches_cpython_marshal(&source, "module_annotation_origins.py");
            }
        }
    }
}

#[test]
fn deferred_module_annotations_keep_their_constant_table_position() {
    for name in ["ω", "名", "name"] {
        for tail in [
            format!(
                "while True:\n    if condition:\n        saved = '{name}'\n        break\n    else:\n        break\n"
            ),
            format!("try:\n    work()\nfinally:\n    saved = '{name}'\n"),
        ] {
            let source =
                format!("def previous({name}):\n    return {name}\n{name}: int = 1\n{tail}");
            common::assert_matches_cpython_marshal(
                &source,
                "deferred_annotation_constant_order.py",
            );
        }
    }
}
