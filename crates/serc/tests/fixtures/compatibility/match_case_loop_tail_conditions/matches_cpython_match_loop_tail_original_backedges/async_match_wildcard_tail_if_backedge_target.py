async def run(events, emit, enabled):
    async for event in events:
        match event:
            case Event(kind=Kind.START):
                emit(event)
            case _:
                if enabled:
                    emit(event)
