async def worker(queue, process):
    while True:
        item = await queue.get()
        await prepare()
        try:
            if item is None:
                return
            try:
                result = await process(item.value)
            except Error as error:
                if not item.done():
                    item.set_error(error)
            else:
                if not item.done():
                    item.set_result(result)
            record(item)
        finally:
            queue.task_done()
