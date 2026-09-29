async def poll(worker, clock, pause, report, writer):
    attempt = 0
    started = clock()
    while True:
        attempt += 1
        try:
            responses = await worker()
            if responses:
                break
        except LookupError as error:
            report(error, level=1)
            await pause()
        except Exception as error:
            report(error, level=0)
            await pause()
        if attempt >= 3 and clock() - started > 5:
            responses = [b'unavailable']
            break
    report(responses, level=1)
    writer.write(responses)
    await writer.drain()
