async def f(flag):
    if flag:
        raise Error
        await produce('operand')
    consume('later')
    yield values
