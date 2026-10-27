use crate::common;

#[test]
fn matches_cpython_borrows_after_shared_conditional_try_exits() {
    for prefix in ["", "async "] {
        for condition in ["flag", "flag and value", "flag or value"] {
            for returned in ["value", "value.attr"] {
                let source = format!(
                    "{prefix}def probe(values, flag, work, use):\n    for item in values:\n        try:\n            value = work(item)\n            if {condition}:\n                return {returned}\n        except Exception:\n            pass\n        use(item)\n"
                );
                common::assert_matches_cpython_marshal(&source, "shared_conditional_try_exit.py");
            }
        }
    }
}

#[test]
fn looping_handler_keeps_the_guarded_else_borrow_successor() {
    for exit in ["continue", "break", "return key", "raise Error()"] {
        let source = format!(
            "def visit(key, items, flag):\n    while flag:\n        try:\n            work()\n        except Error:\n            for item in items:\n                use(item)\n        else:\n            if flag:\n                {exit}\n        use(key.name)\n"
        );
        common::assert_matches_cpython_marshal(&source, "guarded_else_after_handler_loop.py");
    }
}

#[test]
fn a_loop_in_a_recovery_arm_keeps_the_normal_else_borrow_edge() {
    let source = "def visit(key, items, flag):
    while flag:
        try:
            work()
        except Error:
            if flag:
                work()
            else:
                while active():
                    use(key)
        else:
            if flag:
                continue
        use(key.name)
";
    common::assert_matches_cpython_marshal(source, "loop_in_recovery_arm.py");
}
