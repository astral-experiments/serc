async def run(flag):
    try:
        await work()
    finally:
        try:
            if flag:
                await close()
        except Exception:
            pass
