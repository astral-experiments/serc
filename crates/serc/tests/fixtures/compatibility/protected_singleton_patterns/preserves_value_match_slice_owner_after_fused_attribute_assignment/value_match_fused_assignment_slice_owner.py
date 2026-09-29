async def update(event, state, new):
    match (new, event):
        case (True, Entry(role=Role.ASSISTANT, recipient=recipient)) if recipient == 'user':
            await state.begin()
        case (False, Entry(role=Role.ASSISTANT, recipient=recipient)) if recipient == 'user':
            current = state.item
            assert isinstance(current, Item)
            assert state.part is not None
            status = classify(event)
            match status:
                case Status.ACTIVE:
                    previous = state.part.text
                    content = render(event, 0)
                    state.part.text = content
                    delta = content[len(previous):]
                    if delta:
                        await state.append(delta)
                case Status.COMPLETE | Status.PARTIAL:
                    await state.finish(current)
                    state.part = None
                case _:
                    impossible(status)
