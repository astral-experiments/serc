use crate::common;

#[test]
fn matches_cpython_named_elif_cleanup_locations() {
    for (filename, source) in [
        (
            "named_elif_cleanup_before_loop_continuation.py",
            r"def f(items, strict, silent, call, after):
    for item in items:
        try:
            call(item)
        except Exception as error:
            if strict:
                raise
            elif not silent:
                call(error)
        after(item)
",
        ),
        (
            "named_elif_cleanup_before_continuation.py",
            r"def f(strict, silent, call, after):
    try:
        call()
    except Exception as error:
        if strict:
            raise
        elif not silent:
            call(error)
    after()
",
        ),
        (
            "named_elif_cleanup_multiple_terminating_arms.py",
            r"def f(strict, returning, silent, call, after):
    try:
        call()
    except Exception as error:
        if strict:
            raise
        elif returning:
            return error
        elif not silent:
            call(error)
    after()
",
        ),
        (
            "named_elif_cleanup_before_split_loop_condition.py",
            r"def f(items, strict, silent, progress, interval, call, after):
    for item in items:
        try:
            call(item)
        except ValueError as error:
            if strict:
                raise
            elif not silent:
                call(error)
        else:
            try:
                call(item)
            except Exception as error:
                if strict:
                    if not silent:
                        call(error)
                    raise
                elif not silent:
                    call(error)
        if progress and item % interval == 0:
            after(item)
",
        ),
        (
            "named_elif_cleanup_loop_tail_control.py",
            r"def f(items, strict, silent, call):
    for item in items:
        try:
            call(item)
        except Exception as error:
            if strict:
                raise
            elif not silent:
                call(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_elif_cleanup_before_return() {
    for (filename, source) in [
        (
            "matches_cpython_named_elif_cleanup_before_return_handler10.py",
            r"def f(ignore_errors, verbose, data):
    try:
        work()
    except Exception as error:
        message = 'failed'
        if not ignore_errors:
            raise ValueError(message)
        elif verbose:
            report(message)
    return data
",
        ),
        (
            "matches_cpython_named_elif_cleanup_before_return_handler11.py",
            r"def f(ignore_errors, verbose, data):
    try:
        work()
    except Exception as error:
        message = 'failed'
        if not ignore_errors:
            raise ValueError(message)
        elif verbose:
            report(message)
    return None
",
        ),
        (
            "matches_cpython_named_elif_cleanup_before_return_handler12.py",
            r"def f(ignore_errors, verbose, data):
    try:
        work()
    except Exception as error:
        message = 'failed'
        if not ignore_errors:
            raise ValueError(message)
        elif verbose:
            report(message)
    after()
    return data
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
