use crate::common;

#[test]
fn matches_cpython_empty_try_else_declaration_donors() {
    for body in [
        "item: Kind",
        "(item): annotation_effect()",
        "item: Kind\nother: Other",
        "global result",
        "pass\nitem: Kind",
        "item: Kind\npass",
        "0\nitem: Kind",
        "item: Kind\n0",
        "pass\npass\nitem: Kind",
        "pass\nglobal result",
    ] {
        for terminal in [false, true] {
            let mut source = String::from(
                "def f():\n    try:\n        work()\n    except Error:\n        recover()\n    else:\n",
            );
            for line in body.lines() {
                source.push_str("        ");
                source.push_str(line);
                source.push('\n');
            }
            if !terminal {
                source.push_str("    finish()\n");
            }
            common::assert_matches_cpython_marshal(&source, "empty_try_else_declarations.py");
        }
    }
}

#[test]
fn matches_cpython_try_else_annotation_instruction_donors() {
    for body in [
        "item: Kind = produce()",
        "item: Kind = produce()\nother: Other",
        "target.attr: annotation_effect()",
        "target[index()]: annotation_effect()",
        "target.attr: annotation_effect() = produce()",
        "target[index()]: annotation_effect() = produce()",
        "pass",
        "0",
        "0\n1",
    ] {
        let mut source = String::from(
            "def f(target):\n    try:\n        work()\n    except Error:\n        recover()\n    else:\n",
        );
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("    finish()\n");
        common::assert_matches_cpython_marshal(&source, "try_else_annotation_instructions.py");
    }
}

#[test]
fn matches_cpython_empty_try_body_declaration_donors() {
    for body in [
        "item: Kind",
        "global result",
        "pass\nitem: Kind",
        "0\nitem: Kind",
        "item: Kind = produce()",
        "target.attr: annotation_effect()",
    ] {
        for has_else in [false, true] {
            let mut source = String::from("def f(target):\n    try:\n");
            for line in body.lines() {
                source.push_str("        ");
                source.push_str(line);
                source.push('\n');
            }
            source.push_str("    except Error:\n        recover()\n");
            if has_else {
                source.push_str("    else:\n        other: Other\n");
            }
            source.push_str("    finish()\n");
            common::assert_matches_cpython_marshal(&source, "empty_try_body_declarations.py");
        }
    }
}
