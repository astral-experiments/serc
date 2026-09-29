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
                try:
                    await cleanup()
                except ValueError:
                    pass
        except Exception:
            report()
