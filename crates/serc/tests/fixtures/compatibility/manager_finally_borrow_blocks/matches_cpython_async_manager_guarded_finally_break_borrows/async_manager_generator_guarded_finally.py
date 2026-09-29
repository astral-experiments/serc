async def f(manager, state, receive):
    async with manager:
        try:
            while True:
                item = await receive()
                if item is None:
                    break
                state.record(item)
                if item.failed:
                    state.mark()
                yield item
        finally:
            if state.task is not None and not state.task.done():
                if state.queue is not None:
                    state.queue.put_nowait(None)
