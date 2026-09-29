async def run(manager, work, ready, retry, done, report):
    async with manager():
        try:
            while True:
                value = await work()
                if not retry(value):
                    break
                while True:
                    error = None
                    try:
                        state = await ready()
                    except Exception as caught:
                        error = caught
                    if error is None:
                        report(state)
                        break
                    await work()
            events = value.get("events")
            if events is not None:
                report(events)
            return value
        finally:
            done()
