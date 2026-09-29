async def receive(reader, report, close):
    try:
        buffer = bytearray()
        while True:
            chunk = await reader.read(64)
            if not chunk:
                break
            buffer.extend(chunk)
            report(buffer)
        if buffer:
            report(bytes(buffer))
    except Exception as error:
        report(error)
    finally:
        await close()
