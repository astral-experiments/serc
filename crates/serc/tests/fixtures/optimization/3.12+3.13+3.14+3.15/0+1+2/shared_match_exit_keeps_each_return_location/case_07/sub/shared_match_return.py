def enclosing():
    def f(value, flag, work, manager):
        with manager:
            if flag:
                match value:
                    case int():
                        result = 1
                global marker
                assert flag
    return f
f = enclosing()
