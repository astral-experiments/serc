use crate::common;

#[test]
fn copies_while_backedges_across_nested_guard_exits() {
    for branch in [
        "if other:\n    emit()\nelse:\n    if ready:\n        work()",
        "try:\n    read()\nexcept Error:\n    break\nelse:\n    if ready:\n        work()",
    ] {
        let mut source = String::from(
            "def f(active, other, ready, stop):\n    while True:\n        if active:\n",
        );
        for line in branch.lines() {
            source.push_str("            ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("        elif stop:\n            break\n");
        common::assert_matches_cpython_marshal(&source, "nested_guard_loop_exit.py");
    }
}

#[test]
fn matches_cpython_terminal_try_else_assertion_donors() {
    for condition in ["a() and b()", "not a()", "a() == b() and c() is target"] {
        for handler in ["pass", "return"] {
            let source = format!(
                "def f():\n    try:\n        work()\n    except Error:\n        {handler}\n    else:\n        assert {condition}\n"
            );
            common::assert_matches_cpython_marshal(&source, "terminal_try_else_assert.py");
        }
    }
}

#[test]
fn matches_cpython_terminal_try_else_break_donors() {
    for tail in [
        "for item in items:\n    if check(item):\n        consume(item)\n        break\n",
        "for item in items:\n    if check(item):\n        break\nelse:\n    consume()\n",
        "for item in items:\n    break\n",
        "for item in items:\n    consume(item)\n",
    ] {
        for handler in ["pass", "return"] {
            let mut source = format!(
                "def f():\n    try:\n        work()\n    except Error:\n        {handler}\n    else:\n"
            );
            for line in tail.lines() {
                source.push_str("        ");
                source.push_str(line);
                source.push('\n');
            }
            common::assert_matches_cpython_marshal(&source, "terminal_try_else_break.py");
        }
    }
}
