async def handle(flag, work, finish):
    left = right = None
    await finish()
    try:
        work()
    except Exception:
        note = work()
        if flag and note is not None:
            left = work()
            right = work() if left is not None else None
        else:
            left, right = None, None
    await finish()
    return left, right
