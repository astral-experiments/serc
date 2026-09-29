def enclosing():
    def f(value, flag, work, manager):
        try:
            if flag:
                match value:
                    case int():
                        return
                    case str():
                        result = 2
                global marker
                assert flag
        except Exception:
            work("caught")
    return f
f = enclosing()
