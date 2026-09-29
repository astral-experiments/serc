def make_worker(processed, frequency):
    cursor = 0

    async def process(idx, manager, work):
        nonlocal cursor
        with manager():
            mark = False
            try:
                await work()
                mark = True
            except (Cancelled, Killed):
                raise
            except Exception:
                log()
                mark = True
            finally:
                processed.add(idx)
                while (cursor + 1) in processed:
                    cursor += 1
                    if cursor % frequency == 0:
                        heartbeat(cursor)
                        increment()

    return process
