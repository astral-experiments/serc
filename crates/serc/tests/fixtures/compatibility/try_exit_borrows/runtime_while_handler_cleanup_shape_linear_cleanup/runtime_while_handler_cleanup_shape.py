def fill(state):
    try:
        while not state.stopped():
            try:
                item = next(state.iterator)
            except StopIteration:
                state.buffer.put(STOP)
                break
            state.buffer.put(item)
    except Exception as error:
        if not state.stopped():
            state.buffer.put(wrap(error))
            state.buffer.put(STOP)
    finally:
        state.iterator.close()
