async def recover(flag):
    try:
        await work()
    except LookupError:
        if flag:
            first()
        else:
            second()
        pass
