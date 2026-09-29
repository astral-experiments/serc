async def observe(record, state):
    match record:
        case Entry(kind=Mode.START, tool='cache'):
            await state.begin()
        case Entry(kind=Mode.STOP, tool='cache'):
            if isinstance(state.item, Pending) and state.item.status == Status.ACTIVE:
                await state.item.complete()
        case _:
            pass
