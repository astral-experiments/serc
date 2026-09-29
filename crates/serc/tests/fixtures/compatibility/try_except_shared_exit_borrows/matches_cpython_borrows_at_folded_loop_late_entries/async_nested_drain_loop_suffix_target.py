async def receive(reader, report, close):
    try:
        buffer = bytearray()
        while True:
            chunk = await reader.read(64)
            if not chunk:
                break
            buffer.extend(chunk)
            while True:
                split = buffer.find(b"\n")
                if split < 0:
                    break
                line = bytes(buffer[:split])
                del buffer[:split + 1]
                report(line)
        if buffer:
            report(bytes(buffer))
    except Exception as error:
        report(error)
    finally:
        await close()
