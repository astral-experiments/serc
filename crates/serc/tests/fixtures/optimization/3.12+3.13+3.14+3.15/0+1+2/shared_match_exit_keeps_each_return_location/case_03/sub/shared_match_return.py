def enclosing():
    cell = 1
    def f(value, flag, work):
        yield 1
        if flag:
            match value:
                case 1:
                    result = 1
                case 2:
                    result = 2
                case _:
                    result = 3
            nonlocal cell
            assert flag
    return f
f = enclosing()
