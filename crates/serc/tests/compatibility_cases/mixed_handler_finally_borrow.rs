use crate::common::assert_matches_cpython_marshal;

#[test]
fn guarded_named_handlers_reach_linear_finally_cleanup() {
    for scope in ["def", "async def"] {
        for guard in ["x", "x and y", "x or y", "x is None"] {
            for body in ["pass", "work(x)"] {
                for exit in ["raise", "raise F from error", "return x"] {
                    for cleanup in ["x.field += 1", "use(x)", "x.field = y"] {
                        for prefix in ["", "except E:\n        pass\n    "] {
                            let source = format!(
                                "{scope} f(x, y):\n    try:\n        {body}\n    {prefix}except F as error:\n        if {guard}:\n            {exit}\n    finally:\n        {cleanup}\n"
                            );
                            assert_matches_cpython_marshal(
                                &source,
                                "guarded_named_handler_finally.py",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_plain_and_guarded_handlers_finally_borrow() {
    assert_matches_cpython_marshal(
        r"from __future__ import annotations

def run_task(worker: Worker) -> None:
    state = State()
    previous_first = registry.get(registry.FIRST)
    previous_second = registry.get(registry.SECOND)

    def handle_event(code: int, frame: object) -> None:
        state.count += 1
        state.active = True
        event_name = registry.Label(code).name
        if state.count == 1:
            print(
                f'Event {event_name}; requesting a retry. '
                'Another event cancels the request.'
            )
            if request_retry():
                return
            print('The request is unavailable.')
            raise ValueError

        print(f'Repeated event {event_name}; cancelling the request.')
        finish()
        raise ValueError

    registry.set(registry.FIRST, handle_event)
    registry.set(registry.SECOND, handle_event)

    try:
        worker.run()
    except ValueError:
        state.active = True
    except Exception:
        if not state.active:
            raise
    finally:
        registry.set(registry.FIRST, previous_first)
        registry.set(registry.SECOND, previous_second)
        if state.active:
            finish()

    if state.active:
        raise RuntimeError(7)
",
        "mixed_handler_finally_borrow.py",
    );
}

#[test]
fn mixed_pass_and_guarded_handlers_finally_borrow_control() {
    assert_matches_cpython_marshal(
        r"from __future__ import annotations

def run_task(worker: Worker) -> None:
    state = State()
    previous_first = registry.get(registry.FIRST)
    previous_second = registry.get(registry.SECOND)

    def handle_event(code: int, frame: object) -> None:
        state.count += 1
        state.active = True
        event_name = registry.Label(code).name
        if state.count == 1:
            print(
                f'Event {event_name}; requesting a retry. '
                'Another event cancels the request.'
            )
            if request_retry():
                return
            print('The request is unavailable.')
            raise ValueError

        print(f'Repeated event {event_name}; cancelling the request.')
        finish()
        raise ValueError

    registry.set(registry.FIRST, handle_event)
    registry.set(registry.SECOND, handle_event)

    try:
        worker.run()
    except ValueError:
        pass
    except Exception:
        if not state.active:
            raise
    finally:
        registry.set(registry.FIRST, previous_first)
        registry.set(registry.SECOND, previous_second)
        if state.active:
            finish()

    if state.active:
        raise RuntimeError(7)
",
        "mixed_handler_finally_pass_control.py",
    );
}
