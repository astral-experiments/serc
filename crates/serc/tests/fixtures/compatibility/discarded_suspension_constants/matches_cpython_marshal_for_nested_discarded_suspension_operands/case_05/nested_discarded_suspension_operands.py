async def f(flag):
    if flag:
        raise Error
        yield
    consume('later')
    yield values
