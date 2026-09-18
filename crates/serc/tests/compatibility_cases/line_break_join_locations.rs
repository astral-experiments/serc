use crate::common;

#[test]
fn preserves_nested_break_join_locations() {
    for (filename, source) in [
        (
            "multi_false_break_join.py",
            r"def f(items, mode, first, second, third):
    for item in items:
        if mode == 0:
            if first and second:
                value = item
                break
        elif mode == 1:
            if first and second and third:
                value = item
                break
        elif mode == 2:
            if first and (second or not third):
                value = item
                break
        else:
            raise ValueError(mode)
        consume(item)
    return value
",
        ),
        (
            "multi_false_break_continue_control.py",
            r"def f(items, mode, first, second):
    for item in items:
        if mode:
            if first and second:
                break
        else:
            continue
        consume(item)
    return None
",
        ),
        (
            "single_false_break_join_control.py",
            r"def f(items, mode, first, second):
    for item in items:
        if mode == 0:
            if first:
                break
        elif mode == 1:
            if first or second:
                break
        else:
            raise ValueError(mode)
        consume(item)
    return None
",
        ),
        (
            "multi_false_break_nonterminal_else_control.py",
            r"def f(items, mode, first, second):
    for item in items:
        if mode:
            if first and second:
                break
        else:
            consume(mode)
        consume(item)
    return None
",
        ),
        (
            "named_cleanup_nested_break_else.py",
            r"def retry(manager, running, accepted, stop):
    with manager:
        while running:
            try:
                action()
            except Error as error:
                record(error)
                if accepted:
                    refresh()
                    if stop:
                        record(error)
                        break
                    else:
                        record(error)
                        pause()
                else:
                    raise error
    finish()
",
        ),
        (
            "named_cleanup_nested_break_false_control.py",
            r"def retry(manager, running, accepted, stop):
    with manager:
        while running:
            try:
                action()
            except Error as error:
                record(error)
                if accepted:
                    refresh()
                    if stop:
                        record(error)
                        break
                else:
                    raise error
    finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
