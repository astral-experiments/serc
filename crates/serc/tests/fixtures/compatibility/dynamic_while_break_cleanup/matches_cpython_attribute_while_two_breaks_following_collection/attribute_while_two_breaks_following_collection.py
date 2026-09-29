class DrainState:
    async def drain(self, *, timeout_seconds):
        deadline = timeout_seconds
        self._collect_completed()
        try:
            while self._pending_leases:
                remaining = deadline - self._clock()
                if remaining <= 0:
                    break
                done = await self._wait(remaining)
                if not done:
                    break
        except Exception:
            raise
        self._collect_completed()
        return self._pending_error()
