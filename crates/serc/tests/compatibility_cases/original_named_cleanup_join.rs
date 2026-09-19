use crate::common;

#[test]
fn named_return() {
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    try:
        return work()
    except Exception as exc:
        with manager:
            return fallback(exc)
",
        "named_return.py",
    );
}

#[test]
fn named_raise() {
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    try:
        work()
    except Exception as exc:
        with manager:
            raise ValueError(exc)
",
        "named_raise.py",
    );
}

#[test]
fn named_nested_with() {
    common::assert_matches_cpython_marshal(
        r"def f(manager, other):
    try:
        return work()
    except Exception as exc:
        with manager:
            with other:
                return fallback(exc)
",
        "named_nested_with.py",
    );
}

#[test]
fn bare_return_control() {
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    try:
        return work()
    except Exception:
        with manager:
            return fallback()
",
        "bare_return_control.py",
    );
}

#[test]
fn named_fallthrough_control() {
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    try:
        work()
    except Exception as exc:
        with manager:
            fallback(exc)
",
        "named_fallthrough_control.py",
    );
}

#[test]
fn named_following_work_control() {
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    try:
        work()
    except Exception as exc:
        with manager:
            fallback(exc)
    finish()
",
        "named_following_work_control.py",
    );
}

#[test]
fn ordinary_with_control() {
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    with manager:
        return work()
",
        "ordinary_with_control.py",
    );
}

#[test]
fn matches_cpython_nested_named_generator_cleanup_joins() {
    for (filename, source) in [
        (
            "nested_named_generator_handler_copy.py",
            r"def run(first, read, reraise):
    try:
        yield read()
    except ValueError as error:
        if first:
            try:
                yield read()
            except TypeError as error:
                if reraise:
                    raise error
                yield read()
        else:
            yield read()
",
        ),
        (
            "nested_named_generator_distinct_name_control.py",
            r"def run(first, read, reraise):
    try:
        yield read()
    except ValueError as error:
        if first:
            try:
                yield read()
            except TypeError as nested:
                if reraise:
                    raise nested
                yield read()
        else:
            yield read()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
