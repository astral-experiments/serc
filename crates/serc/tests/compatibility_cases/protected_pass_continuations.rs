use crate::common;

#[test]
fn protected_passes_keep_authored_continuation_lines() {
    for (filename, source) in [
        (
            "protected_retry_else_nested_if_pass_join.py",
            r"def run(retry, trace, value):
    try:
        while True:
            if retry:
                continue
            else:
                if trace:
                    observe(value)
                pass
            consume(value)
            return value
    except Exception:
        recover()
        raise
",
        ),
        (
            "protected_raise_else_nested_if_pass_join.py",
            r"def run(fail, trace, value):
    try:
        if fail:
            raise ValueError
        else:
            if trace:
                observe(value)
            pass
        consume(value)
        return value
    except Exception:
        recover()
        raise
",
        ),
        (
            "protected_runtime_if_then_pass_join.py",
            r"def run(trace, value):
    try:
        if trace:
            observe(value)
        pass
        consume(value)
        return value
    except Exception:
        recover()
        raise
",
        ),
        (
            "unprotected_retry_else_nested_if_pass_control.py",
            r"def run(retry, trace, value):
    while True:
        if retry:
            continue
        else:
            if trace:
                observe(value)
            pass
        consume(value)
        return value
",
        ),
        (
            "protected_conditional_arm_pass_bypass_control.py",
            r"def run(enter, trace, value):
    try:
        if enter:
            if trace:
                observe(value)
            pass
        consume(value)
        return value
    except Exception:
        recover()
        raise
",
        ),
        (
            "protected_pass_before_normal_try_end_control.py",
            r"def run(trace, value):
    try:
        if trace:
            observe(value)
        pass
    except Exception:
        recover()
        raise
    consume(value)
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
