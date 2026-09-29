async def stream_events(source, observers, signal, pending, stack):
    try:
        tracker = get_tracker()
        await observers.before()
        async for event in source:
            yield event
        for deferred in pending:
            yield deferred
    except Exception as exc:
        await observers.on_error(exc)
        raise
    finally:
        try:
            if signal and not signal.is_set():
                signal.set()
            if tracker:
                tracker.record_before()
            with scope():
                await observers.done()
            if tracker:
                tracker.record_after()
        finally:
            stack.close()
    audit()
