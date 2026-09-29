async def publish(self):
    retry_delay = self.base_delay
    retry_attempt = 1
    while not self.closed:
        await self.pending.wait()
        if self.closed:
            return
        incremental_delay = self.remaining_delay()
        if incremental_delay > 0:
            await pause(incremental_delay)
            if self.closed:
                return
        try:
            published = await self.flush(retry_attempt=retry_attempt)
        except Cancelled:
            raise
        except Superseded as exc:
            retry_delay = self.base_delay
            retry_attempt = 1
            if self.generation == exc.generation:
                report(exc.generation, self.name)
                self.abandon()
        except Exception as exc:
            report(self.name, exc, exc_info=exc)
            self.notify()
            await pause(jitter(retry_delay))
            retry_attempt += 1
            retry_delay = min(self.max_delay, max(self.base_delay, retry_delay * 2))
        else:
            retry_delay = self.base_delay
            retry_attempt = 1
            if not published:
                self.pending.clear()
