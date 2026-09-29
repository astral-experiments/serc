async def drain(source, timeout):
    while True:
        reservation = await source.reserve()
        if not isinstance(reservation, str):
            await source.pause()
            continue
        if source.enabled:
            try:
                async with timeout():
                    result = await source.ack(reservation)
            except Exception:
                source.failures += 1
                source.report_failure()
            else:
                if isinstance(result, dict) and result.get("acked") is True:
                    source.acked += 1
