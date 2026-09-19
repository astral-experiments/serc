use crate::common;

#[test]
fn try_tail_conditional_return() {
    common::assert_matches_cpython_marshal(
        r"def f(flag):
    try:
        if flag:
            return read()
    except Error:
        recover()
    sink(1)
    sink(2)
    return finish()
",
        "try_tail_conditional_return.py",
    );
}

#[test]
fn try_tail_nested_try() {
    common::assert_matches_cpython_marshal(
        r"def f(flag):
    try:
        if flag:
            try:
                consume()
            except InnerError:
                pass
    except OuterError:
        recover()
    sink(1)
    sink(2)
    return finish()
",
        "try_tail_nested_try.py",
    );
}

#[test]
fn try_tail_async_return() {
    common::assert_matches_cpython_marshal(
        r"async def f(flag):
    try:
        if flag:
            return await read()
    except Error:
        return await recover()
    sink(1)
    sink(2)
    raise Missing(flag)
",
        "try_tail_async_return.py",
    );
}

#[test]
fn try_tail_shared_false_edges_control() {
    common::assert_matches_cpython_marshal(
        r"def f(a, b):
    try:
        if a and b:
            return read()
    except Error:
        recover()
    sink(1)
    sink(2)
    return finish()
",
        "try_tail_shared_false_edges_control.py",
    );
}

#[test]
fn try_tail_nested_try_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(flag):
    try:
        value = begin()
        if flag:
            try:
                consume(value)
            except InnerError:
                pass
    except OuterError:
        try:
            recover()
        except InnerError:
            pass
        value = 0
    sink(value)
    sink(2)
    return finish()
",
        "try_tail_nested_try_handler.py",
    );
}

#[test]
fn try_tail_outer_branch_nested_try() {
    common::assert_matches_cpython_marshal(
        r"def f(flag, request):
    value = None
    result = {}
    if not flag:
        try:
            response = request.read()
            if response is not None:
                value = response.status
                if value >= 400:
                    result['error'] = 1
                else:
                    try:
                        result = consume(response.read())
                    except InnerError:
                        result = {}
        except OuterError:
            value = None
    if value is not None and value >= 400:
        sink(value)
    sink(result)
    return finish()
",
        "try_tail_outer_branch_nested_try.py",
    );
}

#[test]
fn try_tail_async_two_returns_named_recovery() {
    common::assert_matches_cpython_marshal(
        r#"async def f(mode):
    try:
        if mode == 1:
            return await read()
        if mode == 2:
            return await consume()
    except Error as error:
        if mode:
            raise
        return await recover(error)
    raise Missing(f"unsupported mode: {mode}")
"#,
        "try_tail_async_two_returns_named_recovery.py",
    );
}
