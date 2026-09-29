async def run(work, cleanup, stop, report):
    while True:
        try:
            try:
                await work()
            except RuntimeError:
                if stop:
                    break
                report()
            finally:
                await cleanup()
        except Exception:
            report()
