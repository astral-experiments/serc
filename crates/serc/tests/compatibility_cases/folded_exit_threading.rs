use crate::common;

#[test]
fn ordinary_finally_exits_thread_through_retained_markers() {
    for source in [
        r#"async def f():
    with manager:
        try:
            async for output in values:
                pass
        except Error:
            pass
        finally:
            try:
                if flag:
                    pass
            finally:
                result = target.context.telemetry
                if result.value is None:
                    result.finalize(reason="done")
"#,
        r#"async def f(value, other, values, flag, manager, target):
    with manager:
        if True:
            consume(value)
        else:
            consume(other)
        try:
            async for item in values:
                consume(item)
        except Error:
            pass
        finally:
            try:
                if flag:
                    pass
            finally:
                result = target.context.telemetry
                if result.value is None:
                    result.finalize(reason="done")
"#,
    ] {
        common::assert_matches_cpython_marshal(source, "ordinary_finally_exit.py");
    }
}
