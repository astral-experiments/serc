use crate::common;

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
