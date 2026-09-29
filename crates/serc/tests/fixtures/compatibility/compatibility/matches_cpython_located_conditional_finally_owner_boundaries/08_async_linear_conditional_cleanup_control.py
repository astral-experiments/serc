async def test(cm, enabled):
    with cm:
        try:
            await work()
        finally:
            if enabled:
                cleanup()
