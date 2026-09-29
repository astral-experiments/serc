async def stream(groups, workers, stop):
    while True:
        if stop.is_set():
            break
        values = []
        async for group in groups:
            with group:
                async for value in group.values():
                    if stop.is_set():
                        return
                    values.append(value)
                    yield value
        if not values:
            return
        if values[-1].done:
            break
        for worker in workers:
            if worker.ready:
                async for value in worker.values():
                    if stop.is_set():
                        return
                    yield value
                break
    yield "done"
