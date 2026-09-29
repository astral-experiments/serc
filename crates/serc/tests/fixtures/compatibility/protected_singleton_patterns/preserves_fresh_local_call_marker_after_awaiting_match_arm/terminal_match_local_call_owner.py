async def observe(record, state, check):
    match record:
        case Entry(kind=Mode.START, tool='cache'):
            await state.begin()
        case Entry(kind=Mode.STOP, tool='cache'):
            if check(state.item):
                await state.item.complete()
        case _:
            pass
