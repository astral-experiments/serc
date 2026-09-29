class Consumer:
    async def finish(self):
        await self.input.put(None)
        try:
            if self.pending is not None:
                result = await self.pending
                if result == Status.FAIL:
                    raise self.error("input")
            while True:
                result = await self.output.get()
                match result:
                    case Status.OK:
                        continue
                    case Status.FAIL:
                        record("failure", extra={"id": self.identifier, "streaming": self.streaming})
                        if not self.streaming:
                            raise self.error("output")
                    case None:
                        break
                    case _:
                        unreachable(result)
        finally:
            await self.task
            await self.logger.flush()
