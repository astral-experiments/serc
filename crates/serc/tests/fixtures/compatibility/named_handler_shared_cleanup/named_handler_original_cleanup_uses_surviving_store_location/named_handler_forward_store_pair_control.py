async def sample(flag, work, report):
    try:
        await work()
    except ValueError as error:
        if flag:
            report(error)
            left = right = None
        else:
            report(error)
            left = right = None
    except Exception:
        left = right = None
    return left, right
