def terminal_nested_while(read, pending, consume):
    while True:
        value = read()
        if value is None:
            break
        while pending():
            consume(value)


def terminal_nested_while_with_following(read, pending, consume):
    while True:
        value = read()
        if value is None:
            break
        while pending():
            consume(value)
    consume(None)
