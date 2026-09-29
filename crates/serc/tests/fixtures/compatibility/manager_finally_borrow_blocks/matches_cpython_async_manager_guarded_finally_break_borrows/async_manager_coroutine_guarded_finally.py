async def f(manager, state, receive, cancel):
    try:
        async with manager:
            pending = None
            other = state.other
            try:
                while True:
                    if pending is None or pending.done():
                        pending = state.start()
                    event = await receive()
                    if event.ready:
                        state.consume(pending)
                        pending = None
                    if event.stopped:
                        message = await other
                        if message is None:
                            break
                        other = state.start()
            finally:
                if pending is not None:
                    await cancel(pending)
                await cancel(other)
    except LookupError:
        state.failed()
    finally:
        if state.open:
            await state.close()
