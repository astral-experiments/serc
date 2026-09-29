def f(flag):
    if flag:
        raise Error('early')
        yield from (lambda: 'child')()
    consume('later')
    yield values
