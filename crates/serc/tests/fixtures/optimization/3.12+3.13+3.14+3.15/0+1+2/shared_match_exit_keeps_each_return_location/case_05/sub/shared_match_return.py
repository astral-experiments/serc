def enclosing():
    cell = 1
    def f(value, flag, work):
        if flag:
            match value:
                case int():
                    result = 1
                case str():
                    result = 2
                case None:
                    result = 3
            work()
    return f
f = enclosing()
