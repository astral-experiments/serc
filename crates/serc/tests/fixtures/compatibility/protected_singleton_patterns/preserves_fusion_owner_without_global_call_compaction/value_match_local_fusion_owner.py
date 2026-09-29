async def update(event, state, render):
    match event.kind:
        case Status.START:
            await state.begin()
        case Status.ACTIVE:
            content = render(event)
            state.part.text = content
            if content:
                await state.append(content)
        case _:
            pass
