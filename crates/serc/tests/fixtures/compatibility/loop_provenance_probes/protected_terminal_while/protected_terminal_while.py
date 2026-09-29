async def protected_terminal_while(read, pending, consume):
    while True:
        value = await read()
        if value is None:
            break
        while pending():
            await consume(value)


def existing_iterator_control(items, action, consume):
    for value in items:
        try:
            action(value)
        finally:
            if value:
                consume(value)
