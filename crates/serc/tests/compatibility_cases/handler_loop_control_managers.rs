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

#[test]
fn matches_cpython_unnamed_handler_break_enclosing_try_owners() {
    for (filename, source) in [
        (
            "named_handler_while_break_outer_try_control.py",
            r"def target(values, work, done):
    while True:
        try:
            try:
                value = next(values)
            except StopIteration as error:
                break
            work(value)
        except ValueError:
            work(None)
            break
        if done():
            break
    for value in ():
        work(value)
",
        ),
        (
            "unnamed_handler_while_break_leaves_outer_try.py",
            r"def target(values, work, done):
    while True:
        try:
            try:
                value = next(values)
            except StopIteration:
                break
            work(value)
        except ValueError:
            work(None)
            break
        if done():
            break
    for value in ():
        work(value)
",
        ),
        (
            "unnamed_handler_while_break_retains_enclosing_try_control.py",
            r"def target(values, work):
    try:
        while True:
            try:
                value = next(values)
            except StopIteration:
                break
            work(value)
        work(None)
    except ValueError:
        work(False)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
