use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_manager_break_exit() {
    for body in [
        "            if flag:\n                break\n            work(item)\n",
        "            work(item)\n        except Error as error:\n            if flag:\n                break\n            report(error)\n",
    ] {
        for prefix in ["", "            prepare(item)\n"] {
            for following in ["", "    finish()\n"] {
                let source = format!(
                    "def f(items, flag, manager):\n    for item in items:\n        try:\n{body}        finally:\n{prefix}            with manager:\n                release(item)\n{following}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_finally_manager_break_exit.py",
                );
            }
        }
    }
}
