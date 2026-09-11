use crate::common;

#[test]
fn matches_cpython_conditional_manager_suppression_path() {
    for (filename, source) in [
        (
            "conditional_nested_manager_return.py",
            r"def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                result = work(value)
                return result
        else:
            result = work(value)
            return result
",
        ),
        (
            "conditional_nested_manager_else_return.py",
            r"def convert(outer, inner, choose, value):
    with outer:
        if choose:
            return value
        else:
            with inner:
                return value
",
        ),
        (
            "conditional_nested_manager_raise.py",
            r"def convert(outer, inner, choose, error, value):
    with outer:
        if choose:
            with inner:
                raise error
        else:
            return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_manager_ownership() {
    for (filename, source) in [
        (
            "enclosing_try_conditional_manager.py",
            r"def convert(outer, inner, choose, value):
    try:
        with outer:
            if choose:
                with inner:
                    return value
            else:
                return value
    except Error:
        return None
",
        ),
        (
            "coroutine_conditional_sync_manager.py",
            r"async def convert(outer, inner, choose, value):
    with outer:
        if choose:
            with inner:
                return value
        else:
            return value
",
        ),
        (
            "outer_loop_conditional_manager.py",
            r"def convert(managers, inner, choose, value):
    for outer in managers:
        with outer:
            if choose:
                with inner:
                    return value
            else:
                return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_manager_controls() {
    for (filename, source) in [
        (
            "both_manager_arms.py",
            r"def convert(outer, left, right, choose, value):
    with outer:
        if choose:
            with left:
                return value
        else:
            with right:
                return value
",
        ),
        (
            "inner_manager_falls_through.py",
            r"def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                work(value)
        else:
            return value
",
        ),
        (
            "multiple_inner_manager_items.py",
            r"def convert(outer, inner, extra, choose, value):
    with outer:
        if choose:
            with inner, extra:
                return value
        else:
            return value
",
        ),
        (
            "async_inner_manager.py",
            r"async def convert(outer, inner, choose, value):
    with outer:
        if choose:
            async with inner:
                return value
        else:
            return value
",
        ),
        (
            "elif_manager_branch.py",
            r"def convert(outer, inner, first, second, value):
    with outer:
        if first:
            return value
        elif second:
            with inner:
                return value
        else:
            return value
",
        ),
        (
            "constant_manager_condition.py",
            r"def convert(outer, inner, value):
    with outer:
        if True:
            with inner:
                return value
        else:
            return value
",
        ),
        (
            "manager_condition_has_following_statement.py",
            r"def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                return value
        else:
            return value
        work(value)
",
        ),
        (
            "two_plain_terminal_arms.py",
            r"def convert(outer, choose, value):
    with outer:
        if choose:
            return value
        else:
            raise Error
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_conditional_finally_while_manager_exit() {
    common::assert_matches_cpython_marshal(
        r"def make_worker(processed, frequency):
    cursor = 0

    async def process(idx, manager, work):
        nonlocal cursor
        with manager():
            mark = False
            try:
                await work()
                mark = True
            except (Cancelled, Killed):
                raise
            except Exception:
                log()
                mark = True
            finally:
                if mark:
                    processed.add(idx)
                    while (cursor + 1) in processed:
                        cursor += 1
                        if cursor % frequency == 0:
                            heartbeat(cursor)
                            increment()

    return process
",
        "conditional_finally_while_manager.py",
    );
}

#[test]
fn matches_cpython_direct_finally_while_manager_exit() {
    common::assert_matches_cpython_marshal(
        r"def make_worker(processed, frequency):
    cursor = 0

    async def process(idx, manager, work):
        nonlocal cursor
        with manager():
            mark = False
            try:
                await work()
                mark = True
            except (Cancelled, Killed):
                raise
            except Exception:
                log()
                mark = True
            finally:
                processed.add(idx)
                while (cursor + 1) in processed:
                    cursor += 1
                    if cursor % frequency == 0:
                        heartbeat(cursor)
                        increment()

    return process
",
        "direct_finally_while_manager.py",
    );
}

#[test]
fn matches_cpython_shared_conditional_finally_manager_exit() {
    for (filename, source) in [
        (
            "shared_conditional_finally_manager_exit.py",
            r"def cleanup(outer, inner, work, exists, remove, value):
    with outer():
        resource = None
        try:
            resource = work(value)
        finally:
            if resource is not None and exists(resource):
                with inner():
                    remove(resource)
    return work(value)
",
        ),
        (
            "shared_conditional_finally_manager_three_edges.py",
            r"def cleanup(outer, inner, work, exists, remove, value, enabled):
    with outer():
        resource = None
        try:
            resource = work(value)
        finally:
            if resource is not None and enabled and exists(resource):
                with inner():
                    remove(resource)
    return work(value)
",
        ),
        (
            "located_chained_comparison_finally_manager_exit.py",
            r"def cleanup(outer, inner, work, remove, low, value, high):
    with outer():
        try:
            work(value)
        finally:
            if low < value < high:
                with inner():
                    remove(value)
    return work(value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
