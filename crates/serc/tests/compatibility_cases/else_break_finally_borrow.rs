use crate::common;

#[test]
fn matches_cpython_marshal_for_else_break_finally_borrow_continuation_control() {
    let source = r#"async def run(state):
    while True:
        delay = state.next_delay()
        if delay <= 0:
            break
        try:
            await scheduler.wait_condition(
                state.interrupted,
                timeout=delay,
                timeout_summary="Waiting before next check",
            )
        except TimeoutError:
            pass
        else:
            if state.done():
                break
            completed()
        finally:
            state.wake.clear()
        if state.stop() or state.skip.is_set():
            break
    return None
"#;
    common::assert_matches_cpython_marshal(
        source,
        "else_break_finally_borrow_continuation_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_else_break_finally_borrow_target() {
    let source = r#"async def run(state):
    while True:
        delay = state.next_delay()
        if delay <= 0:
            break
        try:
            await scheduler.wait_condition(
                state.interrupted,
                timeout=delay,
                timeout_summary="Waiting before next check",
            )
        except TimeoutError:
            pass
        else:
            if state.done():
                break
        finally:
            state.wake.clear()
        if state.stop() or state.skip.is_set():
            break
    return None
"#;
    common::assert_matches_cpython_marshal(source, "else_break_finally_borrow_target.py");
}
