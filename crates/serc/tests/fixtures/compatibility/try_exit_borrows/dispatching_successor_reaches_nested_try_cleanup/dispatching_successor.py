async def f(sink, buffer, sample_rate):
    try:
        while True:
            message = await sink.receive()
            msg_type = message.get("type")
            if msg_type == "disconnect":
                report("disconnected")
                break
            if message.get("text") is not None:
                parsed = await parse(message["text"])
                if not parsed:
                    continue
                event = parsed.get("event")
                if event == "start":
                    maybe_rate = parsed.get("sample_rate")
                    if isinstance(maybe_rate, (int, float)) and maybe_rate > 0:
                        sample_rate = int(maybe_rate)
                    report(sample_rate)
                elif event == "stop":
                    path = save(buffer, sample_rate)
                    await sink.send({"event": "saved", "path": str(path), "bytes": len(buffer)})
                    report(path, len(buffer))
                    break
                continue
            chunk = message.get("bytes")
            if chunk:
                buffer.extend(chunk)
    except Disconnect:
        report("disconnected")
    except Exception:
        report_error()
        try:
            await sink.send({"event": "error"})
        except Exception:
            pass
        raise
    finally:
        try:
            await sink.close()
        except Exception:
            pass
