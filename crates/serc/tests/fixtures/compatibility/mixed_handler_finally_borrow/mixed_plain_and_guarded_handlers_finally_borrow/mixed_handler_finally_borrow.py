from __future__ import annotations

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
