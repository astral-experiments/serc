def terminal_else(flag):
    try:
        if flag:
            action()
        else:
            other()
    except Error:
        failed()
    else:
        return None
    after()
