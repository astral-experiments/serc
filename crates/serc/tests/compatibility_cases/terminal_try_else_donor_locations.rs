use crate::common;

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

#[test]
fn copies_loop_exits_for_both_arms_of_a_conditional_expression() {
    for condition in [
        "compute(item) if ready else item",
        "item if ready else other",
    ] {
        let source = format!(
            "def f(items, ready, other):\n    for item in items:\n        try:\n            if {condition}:\n                return True\n        except Error:\n            return True\n    return False\n"
        );
        common::assert_matches_cpython_marshal(&source, "conditional_try_loop_exit.py");
    }
}
