use crate::common;

#[test]
fn matches_cpython_handler_loop_control_manager_cleanup() {
    for asynchronous in [false, true] {
        for asynchronous_loop in [false, true] {
            if asynchronous_loop && !asynchronous {
                continue;
            }
            for asynchronous_manager in [false, true] {
                if asynchronous_manager && !asynchronous {
                    continue;
                }
                for nested in [false, true] {
                    for handler in ["Error", "Error as exc", ""] {
                        for control in ["break", "continue"] {
                            let scope = if asynchronous { "async " } else { "" };
                            let loop_kind = if asynchronous_loop { "async " } else { "" };
                            let manager_kind = if asynchronous_manager { "async " } else { "" };
                            let await_prefix = if asynchronous { "await " } else { "" };
                            let manager = if nested {
                                format!("{manager_kind}with manager, other:")
                            } else {
                                format!("{manager_kind}with manager:")
                            };
                            let source = format!(
                                "{scope}def f(items, outer, manager, other):
    with outer:
        {loop_kind}for item in items:
            {manager}
                try:
                    {await_prefix}work(item)
                except {handler}:
                    {control}
"
                            );
                            common::assert_matches_cpython_marshal(
                                &source,
                                "handler_loop_control_managers.py",
                            );
                        }
                    }
                }
            }
        }
    }
}
