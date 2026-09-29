async def f(self):
    while True:
        item = await self.queue.get()
        self.busy = True
        retained = False
        try:
            if self.active is not None:
                try:
                    await runtime.shield(self.active)
                except Retry:
                    pass
            async with self.lock:
                await self.start(
                    item.input,
                    item.key,
                    item.destination,
                    item.bound,
                    item.fresh,
                    item.progress,
                )
                if item.resource is not None:
                    self.retain(item.resource)
                    retained = True
                task = self.active
            if task is None:
                raise RuntimeError("missing task")
            response = await runtime.shield(task)
            if not item.future.done():
                item.future.set_result(response)
        except runtime.Cancelled:
            if not item.future.done():
                item.future.cancel()
            raise
        except Exception as error:
            if not item.future.done():
                item.future.set_exception(error)
            if isinstance(error, Retry):
                continue
            if self.failure is not None and not self.failure.done():
                self.failure.set_exception(error)
            return
        finally:
            if item.resource is not None and not retained:
                item.resource.cleanup()
            self.busy = False
            self.queue.task_done()
