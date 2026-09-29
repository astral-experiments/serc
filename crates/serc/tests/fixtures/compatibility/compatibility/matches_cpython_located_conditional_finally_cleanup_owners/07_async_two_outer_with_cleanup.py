async def test(cm, second, lock, enabled):
    with cm, second:
        try:
            await work()
        except Exception as exc:
            recover(exc)
        finally:
            flush()
            if enabled:
                with lock:
                    collect()
