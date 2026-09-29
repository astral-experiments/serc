async def run(items, ready, action, recover):
    try:
        await action()
        for item in items:
            if ready(item):
                break
        return action()
    except ValueError:
        return recover()
