async def read(self):
    if not self.enabled:
        return await self.primary.read()
    while True:
        if not self.selected:
            primary_read = asyncio.create_task(self.primary.read())
            activation_wait = asyncio.create_task(self.activation.wait())
            try:
                done, _ = await asyncio.wait(
                    {primary_read, activation_wait},
                    return_when=asyncio.FIRST_COMPLETED,
                )
                if activation_wait in done and self.selected:
                    if primary_read.done():
                        with suppress(asyncio.CancelledError, Exception):
                            primary_read.result()
                    continue
                chunk = await primary_read
                return Packet(
                    data=chunk.data,
                    time=chunk.time,
                    speaker=chunk.speaker,
                    enabled=chunk.enabled,
                    epoch=chunk.epoch,
                    generation=chunk.generation,
                    source=chunk.source or "primary",
                )
            finally:
                if not primary_read.done():
                    primary_read.cancel()
                    with suppress(asyncio.CancelledError):
                        await primary_read
                if not activation_wait.done():
                    activation_wait.cancel()
                    with suppress(asyncio.CancelledError):
                        await activation_wait
        return await self.fallback.read()
