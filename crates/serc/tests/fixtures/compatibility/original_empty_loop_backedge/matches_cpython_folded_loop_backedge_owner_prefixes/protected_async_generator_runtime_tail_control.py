async def run(wait, ready):
    try:
        while True:
            await wait()
            if ready:
                yield None
    except ValueError:
        raise
