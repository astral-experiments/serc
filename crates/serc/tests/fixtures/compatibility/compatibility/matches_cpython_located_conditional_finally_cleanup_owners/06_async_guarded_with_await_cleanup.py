async def test(cm, suppressor, task):
    with cm:
        try:
            await work()
        except Exception:
            recover()
            raise
        finally:
            if task is not None:
                task.cancel()
                with suppressor:
                    await task
    done()
