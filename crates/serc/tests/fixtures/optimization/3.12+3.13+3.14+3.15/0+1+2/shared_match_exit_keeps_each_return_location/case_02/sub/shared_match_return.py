def enclosing():
    cell = 1
    def f(value, flag, work):
        if flag:
            match value:
                case int() if flag:
                    result = 1
                case str() | bytes():
                    result = 2
                case None:
                    result = 3
            global marker
            assert flag
        else:
            work()
    return f
f = enclosing()
