def run(factory, first, second, value, consume):
    try:
        value = factory()
    except Error:
        if first:
            value = {}
        else:
            if second:
                value = {}
            else:
                raise
    consume(value)
    return value
