async def handle(flag, work, finish, p0, p1, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, p12, p13, p14, p15, p16, p17, p18, p19):
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
