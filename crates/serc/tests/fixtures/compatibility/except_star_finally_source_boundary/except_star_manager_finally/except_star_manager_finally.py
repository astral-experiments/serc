async def run(self, manager, group):
    with manager:
        await self.ready()
        started = clock.now()
        try:
            async with group as tasks:
                for index in range(self.count):
                    tasks.create_task(self.work(index))
        except* GroupError:
            pass
        finally:
            elapsed = clock.now() - started
            message = f"runtime {elapsed:,.1f}; tokens {self.tokens}"
            if self.errors:
                message += " errors"
            for error in self.errors:
                message += f"{error}: {self.errors[error]}"
            logger.info(message)
    return 0
