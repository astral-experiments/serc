async def target(manager, queue, cleanup, record):
    with manager as token:
        total = 0
        try:
            while True:
                if total == 0:
                    item = await queue.get()
                else:
                    item = await queue.get()
                if isinstance(item, Exception):
                    raise item
                if item is None:
                    break
                total += 1
                yield item
        except Exception:
            record(token)
            raise
        finally:
            await cleanup
        fresh = total + 1
        record(token, total, fresh)
