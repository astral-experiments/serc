async def sample(flag, work, report):
    try:
        await work()
    except ValueError as error:
        if flag:
            report(error)
            (left,
             right) = None, None
        else:
            report(error)
            (left,
             right) = None, None
    except Exception:
        left, right = None, None
    return left, right
