async def f(stop, wait, heartbeat, sink):
    while not stop.is_set():
        try:
            await wait(stop)
        except TimeoutError:
            try:
                await heartbeat()
                if sink is not None:
                    await sink.write()
            except Exception as exc:
                report(exc)
