async def coroutine(obj, future):
    try:
        await future
    finally:
        while obj.value > 0:
            if not obj.step():
                break
    return True


def ordinary(obj):
    try:
        work()
    finally:
        while obj.value > 0:
            if not obj.step():
                break
    return True
