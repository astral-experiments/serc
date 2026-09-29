async def f(manager, permit, state):
    with manager:
        completed = False
        try:
            try:
                async with permit:
                    if not completed:
                        state.pending -= 1
                        completed = True
                    yield state
            finally:
                state.complete()
                record(state)
        finally:
            if not completed:
                state.pending -= 1
                completed = True
