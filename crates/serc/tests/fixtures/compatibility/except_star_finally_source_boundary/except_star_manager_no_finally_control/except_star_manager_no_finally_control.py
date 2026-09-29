async def run(self, manager, group):
    with manager:
        await self.ready()
        try:
            async with group as tasks:
                for index in range(self.count):
                    tasks.create_task(self.work(index))
        except* GroupError:
            pass
        message = self.message
        if self.errors:
            message += " errors"
        logger.info(message)
    return 0
