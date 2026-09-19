use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_call_returns() {
    let cases = [
        (
            "call_positive.py",
            r"
def call_positive(flag):
    try:
        return produce()
    finally:
        if flag:
            consume()
",
        ),
        (
            "call_control_following.py",
            r"
def call_control_following(flag):
    try:
        result = produce()
    finally:
        if flag:
            consume()
    return result
",
        ),
        (
            "name_return_control.py",
            r"
def name_return_control(flag):
    result = produce()
    try:
        return result
    finally:
        if flag:
            consume()
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_shared_normal_try_computed_returns() {
    for (filename, source) in [
        (
            "normal_try_shared_attribute_return_target.py",
            r"async def run(flag, work, cleanup):
    try:
        if flag:
            try:
                await work()
            finally:
                cleanup()
        else:
            await work()
    except RuntimeError:
        return Result.FAILED
    return Result.READY
",
        ),
        (
            "normal_try_shared_local_return_control.py",
            r"async def run(flag, work, cleanup, result):
    try:
        if flag:
            try:
                await work()
            finally:
                cleanup()
        else:
            await work()
    except RuntimeError:
        return Result.FAILED
    return result
",
        ),
        (
            "normal_try_shared_zero_arg_linear_control.py",
            r"async def run(enabled, ready, work, first, second, missing, report):
    if enabled:
        try:
            await work()
            report()
        except RuntimeError:
            report()
    return Result()
",
        ),
        (
            "normal_try_shared_zero_arg_return_target.py",
            r"async def run(enabled, ready, work, first, second, missing, report):
    if enabled:
        try:
            await work()
            if ready:
                if first:
                    second()
                else:
                    missing()
            else:
                report()
        except RuntimeError:
            report()
    return Result()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
