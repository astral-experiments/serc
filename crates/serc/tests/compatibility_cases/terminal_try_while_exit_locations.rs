use crate::common;

#[test]
fn terminal_try_while_exit_location_target() {
    let source = r"def drain(state):
    try:
        while not state.stopped or not state.pending.empty():
            try:
                items = state.pending.get(timeout=5)
            except queue.Empty:
                continue
            for key, item in items:
                value = item.value
                try:
                    value = int(value)
                except:
                    continue
                state.record(key=key, value=value)
    except Exception as error:
        with state.lock:
            state.error = error
";
    common::assert_matches_cpython_marshal(source, "terminal_try_while_exit_target.py");
}

#[test]
fn terminal_try_while_exit_location_control() {
    let source = r"def drain(state):
    try:
        while not state.stopped or not state.pending.empty():
            try:
                items = state.pending.get(timeout=5)
            except queue.Empty:
                continue
            for key, item in items:
                value = item.value
                try:
                    value = int(value)
                except:
                    continue
                state.record(key=key, value=value)
    except Exception as error:
        with state.lock:
            state.error = error
    state.finish()
";
    common::assert_matches_cpython_marshal(source, "terminal_try_while_exit_control.py");
}
