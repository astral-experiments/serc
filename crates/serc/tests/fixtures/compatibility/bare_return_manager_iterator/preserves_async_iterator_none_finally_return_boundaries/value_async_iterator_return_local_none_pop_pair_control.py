async def run(stream, stop, enqueue):
    pending = None
    try:
        async for message in stream:
            if stop(message):
                return pending
            if pending is not None:
                await enqueue(pending)
            pending = message
    except ValueError:
        pass
    finally:
        if pending is not None:
            await enqueue(pending)
        await enqueue(None)
