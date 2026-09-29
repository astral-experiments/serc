def terminal_try_else(flag):
    try:
        action()
    except Error:
        pass
    else:
        if flag:
            return produce()
    return ""


def nonterminal_try_else_control(flag):
    try:
        action()
    except Error:
        pass
    else:
        if flag:
            consume()
    return ""


def try_body_control(flag):
    try:
        if flag:
            return produce()
    except Error:
        pass
    return ""
