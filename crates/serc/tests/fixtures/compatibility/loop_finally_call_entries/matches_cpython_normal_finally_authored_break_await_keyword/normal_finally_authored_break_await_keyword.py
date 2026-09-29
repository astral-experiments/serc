async def target(queue, finished, pending, gather, task):
    try:
        while True:
            if not pending:
                if finished and not pending:
                    break
                item = await queue.get()
                pending.append(item)
                continue
            item = pending.pop()
            yield item
    except Exception:
        task.cancel()
        raise
    finally:
        await gather(task, return_exceptions=True)
        if pending:
            await gather(*pending, return_exceptions=True)
