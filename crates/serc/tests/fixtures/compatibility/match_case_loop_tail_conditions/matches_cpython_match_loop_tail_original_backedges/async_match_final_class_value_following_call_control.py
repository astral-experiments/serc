async def run(events, emit):
    async for event in events:
        match event:
            case Event(kind=Kind.START):
                emit(event)
            case Event(kind=Kind.END):
                emit(event)
        emit(None)
