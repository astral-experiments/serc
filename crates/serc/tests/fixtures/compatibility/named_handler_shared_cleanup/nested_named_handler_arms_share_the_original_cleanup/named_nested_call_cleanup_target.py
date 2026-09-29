async def run(operation, manager, ready, choose, enabled, record):
    try:
        with manager:
            result = await operation()
    except ValueError as error:
        result = error
        if ready():
            if choose(error):
                if enabled():
                    record(error)
            else:
                record(error, result)
    await record(result)
    return result
