async def f():
    try:
        1
    finally:
        await close()
