async def run(wait):
    try:
        while True:
            await wait()
            if False:
                yield None
    except ValueError:
        raise
