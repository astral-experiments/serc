def enclosing():
    cell = 1
    async def f(value, flag, work):
        yield 1
        if flag:
            match value:
                case int():
                    result = 1
                case str():
                    result = 2
                case None:
                    result = 3
            assert flag
        else:
            work()
    return f
f = enclosing()
