use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_handler_return_swap_owners() {
    let source = r"def nested_attribute(_type):
    try:
        return _type.__qualname__
    except AttributeError:
        try:
            return _type.__name__
        except AttributeError:
            return repr(_type)


def nested_name(elasticsearch):
    try:
        return elasticsearch._async.transport
    except AttributeError:
        try:
            return elasticsearch.transport
        except AttributeError:
            return elasticsearch


def outer_following(flag, value):
    try:
        if flag:
            return value.attr
    except AttributeError:
        try:
            return value.other
        except AttributeError:
            return value
    return None


def single_handler_name_control(value):
    try:
        return value.attr
    except AttributeError:
        return value


def single_handler_call_control(value):
    try:
        return value.attr
    except AttributeError:
        return repr(value)
";
    common::assert_matches_cpython_marshal(source, "nested_handler_return_swap_owners.py");
}

#[test]
fn unnamed_tuple_return_preserves_two_named_handler_cleanup() {
    for (filename, source) in [
        (
            "triple_handler_named_outer_return.py",
            r#"def f(work, consume, value):
    try:
        return work()
    except TimeoutError as outer:
        try:
            work()
        except LookupError:
            pass
        except PermissionError as middle:
            try:
                work()
            except (ValueError, OSError):
                return (124, value or "", consume(f"{outer} {middle}"))
        return (1, value, consume(outer))
"#,
        ),
        (
            "triple_handler_all_unnamed_control.py",
            r#"def f(work, consume, value):
    try:
        return work()
    except TimeoutError:
        try:
            work()
        except LookupError:
            pass
        except PermissionError:
            try:
                work()
            except (ValueError, OSError):
                return (124, value or "", consume(value))
        return (1, value, consume(value))
"#,
        ),
        (
            "two_handler_named_outer_control.py",
            r#"def f(work, consume, value):
    try:
        return work()
    except TimeoutError as outer:
        try:
            work()
        except (ValueError, OSError):
            return (124, value or "", consume(outer))
        return (1, value, consume(outer))
"#,
        ),
        (
            "triple_handler_named_inner_control.py",
            r#"def f(work, consume, value):
    try:
        return work()
    except TimeoutError as outer:
        try:
            work()
        except LookupError:
            pass
        except PermissionError as middle:
            try:
                work()
            except (ValueError, OSError) as inner:
                return (124, value or "", consume(f"{outer} {middle} {inner}"))
        return (1, value, consume(outer))
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
