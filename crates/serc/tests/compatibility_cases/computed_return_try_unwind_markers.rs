use crate::common;

#[test]
fn matches_cpython_regression_for_computed_try_unwind_markers() {
    {
        let filename = "computed_try_with_conditional_return.py";
        let source = r"def read_last(manager, read, valid, ParseError, OuterError):
    try:
        with manager() as resource:
            try:
                value = read(resource)
                return value if valid(value) else None
            except ParseError:
                return None
    except OuterError:
        return None
";
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_computed_values_for_computed_try_unwind_markers() {
    for (filename, source) in [
        (
            "computed_try_with_name.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        try:
            return value
        except Error:
            return None
",
        ),
        (
            "computed_try_with_call.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        try:
            return build(value)
        except Error:
            return None
",
        ),
        (
            "computed_try_with_attribute.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        try:
            return value.result
        except Error:
            return None
",
        ),
        (
            "computed_try_with_conditional.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        try:
            return value if ready else fallback
        except Error:
            return None
",
        ),
        (
            "computed_try_with_boolean.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        try:
            return value and fallback
        except Error:
            return None
",
        ),
        (
            "computed_try_with_tuple.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        try:
            return (value, fallback)
        except Error:
            return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_lexical_depth_for_computed_try_unwind_markers() {
    {
        let filename = "outer_try_and_loop_before_manager.py";
        let source = r"def f(managers, value, ready, fallback):
    for manager in managers:
        try:
            with manager:
                try:
                    return value if ready else fallback
                except Error:
                    return None
        except Outer:
            raise
";
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_controls_for_computed_try_unwind_markers() {
    for (filename, source) in [
        (
            "computed_return_without_inner_try.py",
            r"def f(manager, value, ready, fallback):
    with manager:
        return value if ready else fallback
",
        ),
        (
            "try_outside_manager.py",
            r"def f(manager, value):
    try:
        with manager:
            return value
    except Error:
        raise
",
        ),
        (
            "literal_try_with_return.py",
            r"def f(manager):
    with manager:
        try:
            return None
        except Error:
            return 1
",
        ),
        (
            "named_handler_return_inside_manager.py",
            r"def f(manager):
    with manager:
        try:
            work()
        except Error as error:
            return error
",
        ),
        (
            "finally_return_inside_manager.py",
            r"def f(manager, value):
    with manager:
        try:
            return value
        finally:
            cleanup()
",
        ),
        (
            "pass_finally_return_inside_manager.py",
            r"def f(manager, value):
    with manager:
        try:
            return value
        finally:
            pass
",
        ),
        (
            "iterator_inside_manager.py",
            r"def f(manager, values):
    with manager:
        for value in values:
            try:
                return value
            except Error:
                raise
",
        ),
        (
            "coroutine_try_with_return.py",
            r"async def f(manager, value):
    with manager:
        try:
            return value
        except Error:
            return None
",
        ),
        (
            "async_manager_return.py",
            r"async def f(manager, value):
    async with manager:
        try:
            return value
        except Error:
            return None
",
        ),
        (
            "generator_try_with_return.py",
            r"def f(manager, value):
    yield value
    with manager:
        try:
            return value
        except Error:
            return None
",
        ),
        (
            "nested_try_return_inside_manager.py",
            r"def f(manager, value):
    with manager:
        try:
            try:
                return value
            except Inner:
                raise
        except Outer:
            return None
",
        ),
        (
            "return_after_exception_group_handler.py",
            r"def f(manager, value):
    with manager:
        try:
            work()
        except* Error:
            record()
        return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
