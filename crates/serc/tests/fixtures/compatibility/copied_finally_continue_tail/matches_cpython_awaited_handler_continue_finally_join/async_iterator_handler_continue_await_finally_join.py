async def run(events, state, process, recover, cleanup):
    async for event in events:
        try:
            match event:
                case 1:
                    try:
                        await process(event)
                    except ValueError:
                        await recover(event)
                        continue
                    finally:
                        if state.value != state.expected:
                            await cleanup(event)
                case _:
                    await process(event)
        except Exception:
            raise
