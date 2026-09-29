def enclosing():
    cell = 1
    async def f(value, flag, work):
        if flag:
            match value:
                case int():
                    result = 1
                case str():
                    result = 2
                case None:
                    result = 3
            assert flag
    return f
f = enclosing()
