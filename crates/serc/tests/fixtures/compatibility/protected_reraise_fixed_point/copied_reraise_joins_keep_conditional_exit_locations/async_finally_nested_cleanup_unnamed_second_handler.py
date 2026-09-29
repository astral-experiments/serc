async def run(fut):
    try:
        return await work()
    finally:
        try:
            fut = start_cleanup()
            await finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                if fut is not None:
                    fut.cancel()
            except Exception:
                pass
        except Exception:
            warn()
