use crate::common;

#[test]
fn matches_cpython_normal_handler_cleanup_copies() {
    for (filename, source) in [
        (
            "normal_handler_cleanup_nested_explicit_else.py",
            r"def probe(work, a, b, flag, inner):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = 2
            if inner:
                a()
            else:
                b()
    work()
",
        ),
        (
            "normal_handler_cleanup_nested_with.py",
            r"def probe(manager, work, a, b, flag, inner):
    with manager:
        try:
            work()
        except ValueError:
            value = 1
            if flag:
                value = 2
                if inner:
                    a()
                else:
                    b()
",
        ),
        (
            "normal_handler_cleanup_nested_async_with_loop.py",
            r"async def probe(manager, work, a, b, flag, inner, items):
    async with manager:
        try:
            await work()
        except ValueError:
            value = 1
            if flag:
                value = 2
                if inner:
                    a()
                else:
                    values = []
                    for item in items:
                        values.append(item)
                    b(values)
",
        ),
        (
            "normal_handler_cleanup_multi_statement_else.py",
            r"def probe(work, a, b, flag):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = a()
            b(value)
        else:
            value = b()
            a(value)
    work()
",
        ),
        (
            "normal_handler_cleanup_suspended_multi_else.py",
            r"async def probe(work, a, b, flag):
    try:
        await work()
    except ValueError:
        await work()
        if flag:
            value = await a()
            b(value)
        else:
            if flag:
                yield 1
            value = await a()
    else:
        b(0)
    b(1)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_conditional_normal_handler_cleanup_joins() {
    for (filename, source) in [
        (
            "normal_handler_cleanup_nested_no_else.py",
            r"def probe(work, a, flag, inner):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = 2
            if inner:
                a()
    work()
",
        ),
        (
            "normal_handler_cleanup_multi_statement_no_else.py",
            r"def probe(work, a, b, flag):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = a()
            b(value)
    work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_normal_cleanup_after_stable_tail() {
    for (filename, source) in [
        (
            "normal_cleanup_stable_tuple_runtime_call.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            left, right = work()
    work()
",
        ),
        (
            "normal_cleanup_stable_comprehension_then_call.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            value = {item: item for item in items}
            first(value)
    work()
",
        ),
        (
            "normal_cleanup_stable_tuple_then_call.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            left, right = None, None
            first(left, right)
    work()
",
        ),
        (
            "normal_cleanup_stable_tuple_in_earlier_arm.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            left, right = None, None
        else:
            first()
            value = work()
    work()
",
        ),
        (
            "normal_cleanup_stable_comprehension_in_earlier_arm.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = {item: item for item in items}
        else:
            first()
            value = work()
    work()
",
        ),
        (
            "normal_cleanup_stable_nested_try_following_call.py",
            r"def probe(work, first, flag):
    result = None
    try:
        work()
    except ValueError:
        first()
        if flag:
            try:
                result = work()
            except ValueError:
                first()
                result = first()
        else:
            first()
            result = first()
    first()
    return result
",
        ),
        (
            "normal_cleanup_stable_flat_small_return.py",
            r"def probe(work, first, flag):
    result = None
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            result = work()
        else:
            first()
            result = first()
    return result
",
        ),
        (
            "normal_cleanup_stable_simple_call_control.py",
            r"def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            first()
            work()
    work()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
