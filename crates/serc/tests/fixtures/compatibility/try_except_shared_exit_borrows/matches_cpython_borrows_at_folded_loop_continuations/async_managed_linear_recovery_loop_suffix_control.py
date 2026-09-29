async def run(manager, work, ready, retry, done, report):
    async with manager():
        try:
            while True:
                value = await work()
                if not retry(value):
                    break
                state = await ready()
                report(state)
            events = value.get("events")
            if events is not None:
                report(events)
            return value
        finally:
            done()
