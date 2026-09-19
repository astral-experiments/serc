use crate::common;

#[test]
fn matches_cpython_nested_try_branch_exit_markers() {
    for action in ["return read()", "consume(flag)", "value = read()"] {
        for handler in ["pass", "consume(flag)", "return flag"] {
            for successor in ["return None", "return flag", "raise Error"] {
                let branch = format!(
                    "if flag:\n    try:\n        {action}\n    except InnerError:\n        {handler}"
                );
                for in_else in [false, true] {
                    let body = if in_else {
                        format!(
                            "try:\n    consume(flag)\nexcept OuterError:\n    consume(flag)\nelse:\n    {}",
                            branch.replace('\n', "\n    "),
                        )
                    } else {
                        format!(
                            "try:\n    {}\nexcept OuterError:\n    consume(flag)",
                            branch.replace('\n', "\n    "),
                        )
                    };
                    let source = format!(
                        "def f(flag):\n    {}\n    {successor}\n",
                        body.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_try_branch_exit_markers.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_shared_branch_locations_across_cleanup_continuations() {
    for (filename, source) in [
        (
            "direct_following_control.py",
            r"def f(a, b, c, manager):
    if a:
        if b:
            sink(1)
        else:
            if c:
                sink(2)
    else:
        sink(3)
        sink(4)
    sink(5)
",
        ),
        (
            "try_cleanup_continuation.py",
            r"def f(a, b, c, manager):
    try:
        if a:
            if b:
                sink(1)
            else:
                if c:
                    sink(2)
        else:
            sink(3)
            sink(4)
    except Exception:
        sink(6)
    sink(5)
",
        ),
        (
            "with_inside_try_continuation.py",
            r"def f(a, b, c, manager):
    try:
        with manager:
            if a:
                if b:
                    sink(1)
                else:
                    if c:
                        sink(2)
            else:
                sink(3)
                sink(4)
    except Exception:
        sink(6)
",
        ),
        (
            "loop_try_continuation.py",
            r"def f(a, b, c, manager):
    while again():
        try:
            if a:
                if b:
                    sink(1)
                else:
                    if c:
                        sink(2)
            else:
                sink(3)
                sink(4)
        except Exception:
            sink(6)
        sink(5)
",
        ),
        (
            "implicit_return_copy_control.py",
            r"def f(a, b, c, manager):
    if a:
        if b:
            sink(1)
        else:
            if c:
                sink(2)
    else:
        sink(3)
        sink(4)
",
        ),
        (
            "located_raising_arm_control.py",
            r"def f(a, b, c, manager):
    try:
        if a:
            if b:
                sink(1)
            else:
                if c:
                    raise RuntimeError
        else:
            sink(3)
            sink(4)
    except Exception:
        sink(6)
    sink(5)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_nested_else_assertion_success_operand_locations() {
    for (filename, source) in [
        (
            "nested_else_assert_not.py",
            r"def f(select, branch, a, b, c):
    if select:
        if branch:
            sink(1)
        else:
            assert not a
    else:
        sink(2)
        sink(3)
    sink(4)
",
        ),
        (
            "nested_else_assert_and.py",
            r"def f(select, branch, a, b, c):
    if select:
        if branch:
            sink(1)
        else:
            assert a is None and b is None and c is None
    else:
        sink(2)
        sink(3)
    sink(4)
",
        ),
        (
            "direct_assert_not_control.py",
            r"def f(select, branch, a, b, c):
    if select:
        assert not a
    else:
        sink(2)
        sink(3)
    sink(4)
",
        ),
        (
            "nested_plain_assert_control.py",
            r"def f(select, branch, a, b, c):
    if select:
        if branch:
            sink(1)
        else:
            assert a
    else:
        sink(2)
        sink(3)
    sink(4)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_recursive_else_terminating_condition_locations() {
    for (filename, source) in [
        (
            "recursive_raise.py",
            r"def f(a, b, c):
    if a:
        if b:
            raise ValueError
        else:
            if c:
                raise RuntimeError
    else:
        sink(1)
        sink(2)
    finish()
",
        ),
        (
            "recursive_continue.py",
            r"def f(items, a, b, c):
    for item in items:
        if a:
            if b:
                continue
            else:
                if not c:
                    continue
        else:
            sink(1)
            sink(2)
        finish(item)
",
        ),
        (
            "recursive_elif_return.py",
            r"def f(a, b, c, d, stop):
    if a:
        if b:
            sink(0)
        else:
            if c:
                sink(1)
            else:
                if d:
                    sink(2)
                elif stop:
                    return
    else:
        sink(3)
        sink(4)
    finish()
",
        ),
        (
            "nonterminating_tail_control.py",
            r"def f(a, b, c):
    if a:
        if b:
            sink(0)
        else:
            if c:
                sink(3)
    else:
        sink(1)
        sink(2)
    finish()
",
        ),
        (
            "shared_false_edge_control.py",
            r"def f(a, b, c, d):
    if a:
        if b:
            raise ValueError
        else:
            if c and d:
                raise RuntimeError
    else:
        sink(1)
        sink(2)
    finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
