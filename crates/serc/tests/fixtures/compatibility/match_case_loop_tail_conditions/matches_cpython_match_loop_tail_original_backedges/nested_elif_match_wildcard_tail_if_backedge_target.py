def run(events, emit, enabled, choose):
    for event in events:
        if event is None:
            emit(None)
        elif choose(event):
            match event:
                case Kind.START:
                    emit(event)
                case _:
                    value = choose(event)
                    if value:
                        emit(event)
