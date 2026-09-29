async def consume(queue, work, close):
    while True:
        item = await queue.get()
        try:
            if item is None:
                return
            try:
                value = await work(item)
            except Exception as error:
                if item.ready:
                    item.reject(error)
                continue
            item.accept(value)
        finally:
            if item is not None:
                close(item)
            queue.task_done()
