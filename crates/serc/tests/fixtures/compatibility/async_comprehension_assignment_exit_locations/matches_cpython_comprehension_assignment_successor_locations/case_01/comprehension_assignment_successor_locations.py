async def probe(streams):
    for stream in streams:
        result = [item async for item in stream]
