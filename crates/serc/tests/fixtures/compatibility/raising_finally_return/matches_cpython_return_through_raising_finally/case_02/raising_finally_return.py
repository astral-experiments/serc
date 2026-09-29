async def f():
    try:
        return 1
    finally:
        raise RuntimeError('cleanup')
