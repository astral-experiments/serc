async def test(cm, lock, enabled):
    async with cm:
        try:
            await work()
        finally:
            if enabled:
                with lock:
                    collect()
