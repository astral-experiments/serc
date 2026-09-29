async def f(flag):
    if flag:
        raise Error
        yield 'operand'
    consume('later')
    yield values
