async def f(sink, cached, scope):
    result = "success"
    try:
        cached = await fetch(scope)
    except Error as error:
        result = "transient" if transient(error) else "failure"
        attempt = clock()
        if cached is None or cached.records is None:
            state = empty(attempt)
            store(state)
            raise Unavailable(scope) from error
        state = replace(cached, attempt)
        store(state)
        warning([part.kind for part in scope], error)
    finally:
        tags = [f"scope:{scope}", f"result:{result}"]
        sink.record(tags)
    return cached
