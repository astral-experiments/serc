async def f(value, manager, work, report, reject):
    with manager:
        match value:
            case First():
                return await work(value)
            case Last():
                return await work(value)
            case _:
                reject(value)
