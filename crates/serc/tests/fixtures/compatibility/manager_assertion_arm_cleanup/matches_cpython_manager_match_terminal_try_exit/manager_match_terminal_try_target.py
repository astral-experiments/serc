async def f(value, manager, work, report, reject):
    with manager:
        match value:
            case First():
                try:
                    return await work(value)
                except:
                    report(value)
                    raise
            case Last():
                return await work(value)
            case _:
                reject(value)
