def f(flag):
    if flag:
        raise Error
        yield from ((yield 'inner') or 'after operand')
    consume('later')
    yield values
