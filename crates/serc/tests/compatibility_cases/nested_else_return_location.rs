use crate::common;

#[test]
fn nested_else_return_location_target() {
    common::assert_matches_cpython_marshal(
        r"def wait_for_result(path, timeout, interval):
    deadline = clock() + timeout
    last_error = None
    last_state = None
    resolved = normalize(path)
    while clock() < deadline:
        try:
            response = query(resolved, 'status')
            if response.get('ok') is not True:
                last_error = str(response.get('error', 'failed'))
            else:
                state = response.get('state')
                if not isinstance(state, dict):
                    last_error = 'missing payload'
                else:
                    last_state = state
                    if failed(state):
                        raise RuntimeError(describe(state))
                    if healthy(state):
                        return state
        except FileNotFoundError:
            last_error = f'absent: {resolved}'
        except RuntimeError:
            raise
        except Exception as exc:
            last_error = str(exc)
        sleep(max(interval, 0.1))
    raise RuntimeError(f'timed out: {last_state!r}; {last_error!r}')
",
        "nested_else_return_location_target.py",
    );
}

#[test]
fn nested_else_return_location_live_tail_control() {
    common::assert_matches_cpython_marshal(
        r"def wait_for_result(path, timeout, interval):
    deadline = clock() + timeout
    last_error = None
    last_state = None
    resolved = normalize(path)
    while clock() < deadline:
        try:
            response = query(resolved, 'status')
            if response.get('ok') is not True:
                last_error = str(response.get('error', 'failed'))
            else:
                state = response.get('state')
                if not isinstance(state, dict):
                    last_error = 'missing payload'
                else:
                    last_state = state
                    if failed(state):
                        raise RuntimeError(describe(state))
                    if healthy(state):
                        return state
            record(response)
        except FileNotFoundError:
            last_error = f'absent: {resolved}'
        except RuntimeError:
            raise
        except Exception as exc:
            last_error = str(exc)
        sleep(max(interval, 0.1))
    raise RuntimeError(f'timed out: {last_state!r}; {last_error!r}')
",
        "nested_else_return_location_live_tail_control.py",
    );
}
