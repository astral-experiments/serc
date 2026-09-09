use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_try_else_pop_block_prefixes() {
    for condition in [
        "value",
        "value < other",
        "value or other",
        "not (value and other)",
        "False or value",
        "True and value",
    ] {
        for action in ["return 1", "raise Error"] {
            for otherwise in ["consume(value)", "pass", "return 2"] {
                for prefix in ["", "async "] {
                    let source = format!(
                        "{prefix}def f(value, other):\n    try:\n        if {condition}:\n            {action}\n    except Error:\n        recover()\n    else:\n        {otherwise}\n    after()\n"
                    );
                    assert_matches_cpython_marshal(&source, "try_else_pop_block_prefix.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_try_else_pop_block_prefixes_inside_loops() {
    for header in ["for item in items:", "while value:"] {
        for condition in ["value", "value < other", "value or other"] {
            for action in ["break", "continue"] {
                let source = format!(
                    "def f(value, other, items):\n    {header}\n        try:\n            if {condition}:\n                {action}\n        except Error:\n            recover()\n        else:\n            consume(value)\n    after()\n"
                );
                assert_matches_cpython_marshal(&source, "try_else_loop_pop_block_prefix.py");
            }
        }
    }
}

#[test]
fn distinguishes_original_loop_exits_before_try_else() {
    for body in [
        "while True:\n    work()",
        "while 1:\n    pass",
        "try:\n    work()\nfinally:\n    while True:\n        work()",
        "for item in items:\n    work(item)\nelse:\n    while True:\n        work()",
        "for item in items:\n    work(item)\nelse:\n    return 1",
    ] {
        for condition in ["value", "value or other"] {
            let mut indented = String::new();
            for line in body.lines() {
                indented.push_str("            ");
                indented.push_str(line);
                indented.push('\n');
            }
            let source = format!(
                "def f(value, other, items):\n    try:\n        if {condition}:\n{indented}    except Error:\n        recover()\n    else:\n        consume(value)\n    after()\n"
            );
            assert_matches_cpython_marshal(&source, "try_else_original_loop_exit.py");
        }
    }
}
