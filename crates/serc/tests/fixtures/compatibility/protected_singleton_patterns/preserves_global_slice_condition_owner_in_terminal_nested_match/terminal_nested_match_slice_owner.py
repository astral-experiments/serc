async def update(event, state):
    match event:
        case Entry(kind='start'):
            await state.begin()
        case Entry(kind='update'):
            match state.item:
                case Item(status=Status.ACTIVE):
                    piece = event.text[len(state.text):]
                    if piece:
                        await state.append(piece)
                case _:
                    pass
        case _:
            pass
