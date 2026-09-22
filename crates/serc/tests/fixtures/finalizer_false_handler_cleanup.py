async def f(active, work, wait, current, ready):
    while active:
        try:
            await work()
        finally:
            try:
                await wait()
            except RuntimeError:
                if current() and ready():
                    raise
            except LookupError:
                pass
