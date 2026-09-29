def run(value, request, retry, report, lookup):
    try:
        request(value)
    except TypeError as outer:
        message = str(outer)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            retry(value)
        except Exception as inner:
            report(inner)
            state = lookup(value)
            if not isinstance(state, State) or state.status != "done":
                report(state)
    except Exception as outer:
        state = lookup(value)
        if not isinstance(state, State) or state.status != "done":
            report(state)
