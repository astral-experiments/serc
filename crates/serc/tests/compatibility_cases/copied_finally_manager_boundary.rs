use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_manager_boundary_target() {
    let source = r"async def run(state):
    attempt = 0
    async with session():
        while attempt < state.limit:
            try:
                await execute(state)
                state.finished = True
                break
            except Cancelled:
                state.cancelled = True
                break
            except Exception:
                attempt += 1
            finally:
                if state.cancelled or state.failed or state.finished:
                    await save(state)
                    state.finished_at = now()
";
    common::assert_matches_cpython_marshal(source, "copied_finally_manager_tail_target.py");
}

#[test]
fn matches_cpython_marshal_for_copied_finally_manager_boundary_continuation_control() {
    let source = r"async def run(state):
    attempt = 0
    async with session():
        while attempt < state.limit:
            try:
                await execute(state)
                state.finished = True
                break
            except Cancelled:
                state.cancelled = True
                break
            except Exception:
                attempt += 1
            finally:
                if state.cancelled or state.failed or state.finished:
                    await save(state)
                    state.finished_at = now()
        after_loop()
";
    common::assert_matches_cpython_marshal(
        source,
        "copied_finally_manager_continuation_control.py",
    );
}
