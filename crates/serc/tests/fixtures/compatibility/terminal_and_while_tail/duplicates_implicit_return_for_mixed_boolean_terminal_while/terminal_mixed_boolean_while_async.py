async def f(state, last=False):
    while state.count >= state.limit or (last and state.count > 0):
        await state.stage()
        state.count = state.count - state.limit
