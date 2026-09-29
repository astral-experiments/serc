def drain(state):
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
