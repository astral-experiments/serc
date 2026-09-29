async def summarize(state, client, sink):
    try:
        while state.pending or client.pending:
            remaining = state.remaining()
            if remaining <= 0:
                state.finish()
                break
            batch = await client.fetch()
            for item in batch:
                if item.ready:
                    state.record(item)
            if state.finished:
                break
    except Exception:
        client.cancel()
        raise
    finally:
        if state.finished:
            sink.record(state)
    return state.result
