async def run(value, request, retry, report, lookup):
    try:
        await request(value)
    except TypeError:
        message = str(value)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            await retry(value)
        except Exception as inner:
            await report(inner)
            state = await lookup(value)
            if not isinstance(state, State) or state.status != "done":
                await report(state)
    except Exception:
        state = await lookup(value)
        if not isinstance(state, State) or state.status != "done":
            await report(state)
