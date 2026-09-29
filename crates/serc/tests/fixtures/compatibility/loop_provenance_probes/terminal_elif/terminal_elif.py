def terminal_elif(read, consume):
    while True:
        value = read()
        if value == 0:
            consume(value)
        elif value == 1:
            consume(-value)
        elif value == 2:
            break


def terminal_elif_with_following(read, consume):
    while True:
        value = read()
        if value == 0:
            consume(value)
        elif value == 1:
            consume(-value)
        elif value == 2:
            break
    consume(None)
