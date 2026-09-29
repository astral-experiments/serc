async def run(value, request, retry, report, lookup):
    try:
        await request(value)
    except TypeError as outer:
        message = str(outer)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            await retry(value)
        except Exception as inner:
            await report(inner)
            state = await lookup(value)
            if not isinstance(state, State) or state.status != "done":
                await report(state)
    except Exception as outer:
        state = await lookup(value)
        if not isinstance(state, State) or state.status != "done":
            await report(state)
    return None
