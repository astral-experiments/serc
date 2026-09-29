def f(flag):
    if flag:
        raise Error('early')
        consume(item for item in values)
    consume('later')
    yield values
