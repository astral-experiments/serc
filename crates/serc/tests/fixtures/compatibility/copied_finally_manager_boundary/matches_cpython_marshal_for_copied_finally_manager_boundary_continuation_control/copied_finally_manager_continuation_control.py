async def run(state):
    attempt = 0
    async with session():
        while attempt < state.limit:
            try:
                await execute(state)
                state.finished = True
                break
            except Cancelled:
                state.cancelled = True
                break
            except Exception:
                attempt += 1
            finally:
                if state.cancelled or state.failed or state.finished:
                    await save(state)
                    state.finished_at = now()
        after_loop()
