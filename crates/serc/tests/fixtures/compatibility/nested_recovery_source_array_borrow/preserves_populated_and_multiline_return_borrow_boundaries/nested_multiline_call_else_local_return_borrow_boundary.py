def run(outer, middle, choice, inner, value, consume):
    if outer:
        consume(value)
        if middle:
            consume(value)
            if choice:
                consume(value)
                if inner:
                    consume(value)
                else:
                    consume(
                        value,
                        mode=2,
                    )
            else:
                consume(value)
    else:
        consume(value)
    return value
