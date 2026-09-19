use crate::common;

#[test]
fn preserves_tuple_operands_before_folded_sets() {
    for elements in [
        ["(1, 2)", "(3, 4)", "(5, 6)"],
        ["('a', 'b')", "('c', 'd')", "('e', 'f')"],
        ["((1, 2), 3)", "((3, 4), 5)", "((5, 6), 7)"],
    ] {
        for count in [2, 3] {
            for reverse in [false, true] {
                let mut values = elements[..count].to_vec();
                if reverse {
                    values.reverse();
                }
                let display = format!("{{{}}}", values.join(", "));
                for expression in [display.clone(), format!("value in {display}")] {
                    let source = format!(
                        "def f(value):\n    assert value.a == value.b == value.c\n    result = {expression}\n    work({})\n    return {}\n",
                        values[0],
                        values[count - 1],
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "frozen_set_construction_dependencies.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_duplicate_set_operand_construction() {
    for elements in [
        ["(-0.0, 1)", "(0.0, 1)", "(2.0, 3.0)"],
        ["(False, 'k')", "(0, 'k')", "(0.0, 'k')"],
    ] {
        for reverse in [false, true] {
            let mut values = elements.to_vec();
            if reverse {
                values.reverse();
            }
            let display = format!("{{{}}}", values.join(", "));
            for prefix in [String::new(), format!("    work({display})\n")] {
                for expression in [display.clone(), format!("value in {display}")] {
                    let source = format!(
                        "def f(value):\n    assert value.a == value.b == value.c\n{prefix}    result = {expression}\n    work({})\n    work({})\n    return {}\n",
                        values[0], values[1], values[2],
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "frozen_set_duplicate_dependencies.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_set_operand_admissions_from_discarded_branches() {
    for elements in [
        ["('a', 'b')", "('c', 'd')", "('e', 'f')"],
        ["(-0.0, 1)", "(0.0, 1)", "(2.0, 3.0)"],
        ["(False, 'k')", "(0, 'k')", "(0.0, 'k')"],
        ["(0.0, 'k')", "(-0j, 'k')", "(0j, 'k')"],
    ] {
        for reverse in [false, true] {
            let mut values = elements.to_vec();
            if reverse {
                values.reverse();
            }
            let original = format!("{{{}}}", values.join(", "));
            values.reverse();
            let later = format!("{{{}}}", values.join(", "));
            for expression in [later.clone(), format!("value in {later}")] {
                let source = format!(
                    "def f(value):\n    assert value.a == value.b == value.c\n    if False:\n        work({original})\n    result = {expression}\n    work({})\n    work({})\n    return {}\n",
                    values[0], values[1], values[2],
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "frozen_set_discarded_dependencies.py",
                );
            }
        }
    }
}
