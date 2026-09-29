def terminal_else_if(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            return flag
    consume(value)


def fallthrough_else_if_control(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            consume(flag)
    consume(value)


def terminal_and_fallthrough_arms(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            return flag
        else:
            other()
    consume(value)


def two_terminal_arms_control(value, flag):
    try:
        action()
    except Exception:
        pass
    else:
        if flag:
            return flag
        else:
            raise RuntimeError
    consume(value)
