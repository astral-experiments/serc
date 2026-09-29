async def run(mode, items, ready, action, recover):
    try:
        match mode:
            case 0:
                await action()
                for item in items:
                    if ready(item):
                        break
            case _:
                await action()
        return action()
    except ValueError:
        return recover()
