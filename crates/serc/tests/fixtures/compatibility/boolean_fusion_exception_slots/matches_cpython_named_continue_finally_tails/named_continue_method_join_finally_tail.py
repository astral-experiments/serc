class Worker:
    async def consume(self):
        while True:
            item = await self.queue.get()
            try:
                if item is None:
                    return
                try:
                    value = await item.run()
                except Exception as error:
                    if not item.future.done():
                        item.future.fail(error)
                    continue
                if not item.future.done():
                    item.future.finish(value)
            finally:
                if item is not None:
                    self.active -= 1
                self.queue.task_done()
