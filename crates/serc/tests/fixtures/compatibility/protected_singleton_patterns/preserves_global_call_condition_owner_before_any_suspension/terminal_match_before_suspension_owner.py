async def observe(record, state):
    match record:
        case Entry(kind=Mode.START, tool='cache'):
            state.begin()
        case Entry(kind=Mode.STOP, tool='cache'):
            if isinstance(state.item, Pending):
                await state.item.complete()
        case _:
            pass
