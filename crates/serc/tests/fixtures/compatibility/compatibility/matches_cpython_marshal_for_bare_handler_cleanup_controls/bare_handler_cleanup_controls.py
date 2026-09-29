def implicit_fallthrough(flag):
    try:
        action()
    except Error:
        if flag:
            raise
    after()


def single_normal_exit(flag):
    try:
        action()
    except Error:
        if flag:
            raise
        else:
            recover()
    after()
