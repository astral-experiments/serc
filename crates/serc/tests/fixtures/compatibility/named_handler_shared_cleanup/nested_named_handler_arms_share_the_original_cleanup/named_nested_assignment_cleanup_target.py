async def run(operation, ready, choose, record):
    result = None
    try:
        result = operation()
    except ValueError as error:
        record(error)
        token = operation()
        if token and ready(token):
            fallback = operation()
            if choose(fallback):
                result = error.value
            else:
                result = fallback
    await record(result)
    return result
