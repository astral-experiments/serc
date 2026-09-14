use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_fresh_condition_markers_after_duplicate_stores() {
    for (filename, source) in [
        (
            "fresh_owner_local_iterable.py",
            r"def f(sock):
    try:
        readable, _, _ = sock
        if not readable:
            return False
        return False
    except OSError:
        return True
",
        ),
        (
            "fresh_owner_local_call.py",
            r"def f(sock, work):
    try:
        readable, _, _ = work()
        if not readable:
            return False
        return False
    except OSError:
        return True
",
        ),
        (
            "fresh_owner_global_attribute.py",
            r"def f(sock):
    try:
        readable, _, _ = select.value
        if not readable:
            return False
        return False
    except OSError:
        return True
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_condition_markers_after_slot_removals() {
    for (filename, source) in [
        (
            "fresh_owner_global_call.py",
            r"def f(sock):
    try:
        readable, _, _ = work()
        if not readable:
            return False
        return False
    except OSError:
        return True
",
        ),
        (
            "fresh_owner_unique_stores.py",
            r"def f(sock):
    try:
        readable, a, b = select.select([sock], [], [], 0)
        if not readable:
            return False
        return False
    except OSError:
        return True
",
        ),
        (
            "fresh_owner_none_condition.py",
            r"def f(sock):
    try:
        readable, _, _ = select.select([sock], [], [], 0)
        if readable is None:
            return False
        return False
    except OSError:
        return True
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_condition_markers_after_earlier_try() {
    let source = r"def f(sock):
    try:
        pass
    finally:
        pass
    try:
        readable, _, _ = select.select([sock], [], [], 0)
        if not readable:
            return False
        return False
    except OSError:
        return True
";
    assert_matches_cpython_marshal(source, "fresh_owner_earlier_empty_try.py");
}

#[test]
fn matches_cpython_condition_marker_preserves_outer_finally_owner() {
    let source = r"def f(flag,a,b):
    try:
        work()
        work()
        work()
        try:
            if flag:
                cleanup()
        finally:
            finish()
    finally:
        outer()
";
    assert_matches_cpython_marshal(source, "fresh_owner_outer_finally.py");
}

#[test]
fn matches_cpython_async_function_for_break_try_expr() {
    assert_matches_cpython_marshal(
        r"async def f(values):
    value = None
    for item in values:
        if consume(item):
            value = item
            break
    try:
        result = finish(value) if value else None
    except Error:
        return None
    return result
",
        "async_function_for_break_try_expr.py",
    );
}

#[test]
fn matches_cpython_async_function_for_break_try_assert() {
    assert_matches_cpython_marshal(
        r#"async def f(values):
    value = None
    for item in values:
        if consume(item):
            value = item
            break
    try:
        assert value, "missing"
        return finish(value)
    except Error:
        return None
"#,
        "async_function_for_break_try_assert.py",
    );
}

#[test]
fn matches_cpython_fresh_try_call_condition_after_branch_join() {
    for (filename, source) in [
        (
            "fresh_try_call_after_join_with_continuation.py",
            r"def f(values, flag):
    for value in values:
        if flag:
            prepare(value)
        else:
            fallback(value)
        try:
            if isinstance(value.payload, Kind):
                consume(value)
            else:
                discard(value)
        except Error:
            recover(value)
        after(value)
",
        ),
        (
            "fresh_try_call_after_guard_with_continuation.py",
            r"def f(values, flag):
    for value in values:
        if flag:
            raise Error(value)
        try:
            if isinstance(value, Kind):
                consume(value)
            else:
                discard(value)
        except Error:
            recover(value)
        after(value)
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
