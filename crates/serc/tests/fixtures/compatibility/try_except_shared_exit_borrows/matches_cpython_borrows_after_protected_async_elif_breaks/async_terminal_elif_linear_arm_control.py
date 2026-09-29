async def collect(stream, decode, handle, close, report):
    try:
        async for message in stream:
            if message.kind == 1:
                payload = decode(message.data)
                await handle(payload)
            elif message.kind == 2:
                break
    finally:
        report(stream)
        await close(stream)
    report(stream)
    return stream
