from __future__ import annotations

def make_worker(state):
    def worker() -> None:
        if True:
            while True:
                item = state.inbox.get()
                state.count += 1
                if item is None:
                    break

                result = state.process(item)
                state.inbox.task_done()
                if result is not None:
                    state.outbox.put(result)
                else:
                    state.failures += 1
                    if state.stop:
                        print('stopping')
                        break
    return worker
