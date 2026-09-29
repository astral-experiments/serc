async def f(manager, state):
    with manager:
        completed = False
        try:
            yield state
        finally:
            if not completed:
                state.pending -= 1
                completed = True
