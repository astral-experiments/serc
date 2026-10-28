use crate::common;

#[test]
fn guarded_loop_else_preserves_branches_before_its_normal_exit() {
    let source = r"def scan(items, group, active):
    for item in items:
        if enabled(item):
            for value in group:
                if value == item:
                    break
            else:
                if active:
                    if ready(item):
                        prepare(item)
                    record(item)
        else:
            finish(item)
";
    common::assert_matches_cpython_marshal(source, "guarded_loop_else_branch.py");
}

#[test]
fn guarded_inner_loop_else_copies_the_enclosing_backedge() {
    for header in ["for value in group:", "while ready():"] {
        let source = format!(
            "def scan(items, group, visible):\n    for item in items:\n        if enabled(item):\n            {header}\n                if stop():\n                    break\n            else:\n                if visible:\n                    record(item)\n        elif pending(item):\n            finish(item)\n"
        );
        common::assert_matches_cpython_marshal(&source, "guarded_inner_loop_else.py");
    }
}

#[test]
fn terminal_else_does_not_locate_original_normal_try_exit() {
    for condition in ["flag", "flag and matched"] {
        for action in ["break", "continue", "return result", "raise Error()"] {
            for finalizer in ["", "        finally:\n            cleanup()\n"] {
                let source = format!(
                    "def f(flag, matched, result):\n    while pending():\n        try:\n            connect()\n        except Error:\n            recover()\n        else:\n            if {condition}:\n                work()\n                if matched:\n                    break\n            else:\n                work()\n                {action}\n{finalizer}        pause()\n    return result\n"
                );
                common::assert_matches_cpython_marshal(&source, "terminal_else_try_exit.py");
            }
        }
    }
}

#[test]
fn while_else_partial_if_keeps_its_original_join_unlocated() {
    for loop_break in ["", "            if stop():\n                break\n"] {
        for next_case in ["", "            elif other:\n                two()\n"] {
            for last_else in ["", "            else:\n                three()\n"] {
                let source = format!(
                    "def f(mode, flag, other, result):\n    if mode:\n        while pending():\n            work()\n{loop_break}        else:\n            if flag:\n                one()\n{next_case}{last_else}    else:\n        fallback()\n    finish()\n    return result\n"
                );
                common::assert_matches_cpython_marshal(&source, "while_else_join.py");
            }
        }
    }
}

#[test]
fn guarded_for_with_terminal_else_keeps_false_join_line() {
    // Early copying moves every break past the original conditional exit.
    // Its remaining false edge supplies the line of the copied return's NOP.
    for condition in ["guard", "guard(value)", "guard or mode", "guard and mode"] {
        for terminal in ["raise ValueError", "return 0"] {
            let source = format!(
                "def f(mode, guard, items, value):\n    if mode:\n        if {condition}:\n            for item in items:\n                value = item\n                if value:\n                    break\n            else:\n                {terminal}\n    else:\n        value = 0\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "guarded_for_terminal_else.py");
        }
    }
}
