use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_inner_finally_owner_target() {
    let source = r"async def run(value, delayed):
    try:
        while True:
            started = clock()
            try:
                await work(value)
            except Error as error:
                if should_stop(value):
                    return
                report(error)
            finally:
                elapsed = clock() - started
                if elapsed > limit or delayed:
                    if delayed:
                        record(value, context=details())
                    report(value, elapsed)
    except Cancelled:
        raise
    finally:
        cleanup(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "copied_inner_finally_outer_try_owner_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_copied_inner_finally_owner_control() {
    let source = r"async def run(value, delayed):
    try:
        while True:
            started = clock()
            try:
                await work(value)
            except Error as error:
                if should_stop(value):
                    return
                report(error)
            finally:
                elapsed = clock() - started
                if elapsed > limit or delayed:
                    if delayed:
                        record(value, context=details())
                    report(value, elapsed)
    finally:
        cleanup(value)
";
    common::assert_matches_cpython_marshal(source, "copied_inner_finally_outer_finally_control.py");
}
