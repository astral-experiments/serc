async def f(self, source, output):
    async with self.manager():
        if self.task is None:
            self.task = start(output)
        try:
            async for item in source:
                match item:
                    case Audio():
                        self.push(item)
                    case Video():
                        await output.push(item)
        finally:
            if self.task:
                log("cancel")
                await cancel(self.task)
