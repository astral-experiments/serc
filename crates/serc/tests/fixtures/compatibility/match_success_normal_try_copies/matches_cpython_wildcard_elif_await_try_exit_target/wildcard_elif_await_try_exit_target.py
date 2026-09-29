async def f(value, mode, work, report, reject):
    result = 0
    try:
        match value:
            case First():
                result = 1
            case Second():
                details = await work(value)
                if mode == "present" and details is not None:
                    result = 1
                elif mode == "absent" and details is None:
                    result = 2
            case Last():
                result = await work(value)
            case _:
                reject(value)
    except Exception as error:
        report(error)
    bias = "up" if result > 1 else ("down" if result < 1 else None)
    return result, bias
