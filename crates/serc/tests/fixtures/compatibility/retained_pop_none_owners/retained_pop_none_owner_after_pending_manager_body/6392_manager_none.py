async def f(state, manager, events, output, log):
    async with manager:
        try:
            async for event in events.consume():
                match event:
                    case int():
                        if state.pending is not None:
                            await state.pending.cancel()
                        state.pending = state.start()
                    case str():
                        if not state.pending:
                            state.pending = state.start()
                        if state.pending:
                            await output.push(state.pending)
                            state.pending = None
        finally:
            if state.pending is not None:
                log()
                await state.pending.cancel()
